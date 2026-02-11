//! `MachineService` gRPC implementation.

use tonic::{Request, Response, Status};
use tracing::{info, instrument};

use betcode_proto::v1::machine_service_server::MachineService;
use betcode_proto::v1::{
    GetMachineRequest, GetMachineResponse, ListMachinesRequest, ListMachinesResponse, MachineInfo,
    MachineStatus, RegisterMachineRequest, RegisterMachineResponse, RemoveMachineRequest,
    RemoveMachineResponse,
};

use crate::server::interceptor::extract_claims;
use crate::storage::RelayDatabase;

pub struct MachineServiceImpl {
    db: RelayDatabase,
}

impl MachineServiceImpl {
    pub const fn new(db: RelayDatabase) -> Self {
        Self { db }
    }
}

fn machine_to_proto(m: &crate::storage::Machine) -> MachineInfo {
    let status = match m.status.as_str() {
        "online" => MachineStatus::Online,
        _ => MachineStatus::Offline,
    };
    MachineInfo {
        machine_id: m.id.clone(),
        name: m.name.clone(),
        owner_id: m.owner_id.clone(),
        status: status as i32,
        registered_at: Some(prost_types::Timestamp {
            seconds: m.registered_at,
            nanos: 0,
        }),
        last_seen: Some(prost_types::Timestamp {
            seconds: m.last_seen,
            nanos: 0,
        }),
        metadata: serde_json::from_str(&m.metadata).unwrap_or_default(),
    }
}

#[tonic::async_trait]
impl MachineService for MachineServiceImpl {
    #[instrument(skip(self, request), fields(rpc = "RegisterMachine"))]
    async fn register_machine(
        &self,
        request: Request<RegisterMachineRequest>,
    ) -> Result<Response<RegisterMachineResponse>, Status> {
        let user_id = {
            let claims = extract_claims(&request)?;
            claims.sub.clone()
        };
        let req = request.into_inner();

        let metadata_json =
            serde_json::to_string(&req.metadata).unwrap_or_else(|_| "{}".to_string());

        let machine = self
            .db
            .create_machine(&req.machine_id, &req.name, &user_id, &metadata_json)
            .await
            .map_err(|e| Status::internal(format!("Failed to register machine: {e}")))?;

        info!(machine_id = %req.machine_id, name = %req.name, "Machine registered");

        Ok(Response::new(RegisterMachineResponse {
            machine: Some(machine_to_proto(&machine)),
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "ListMachines"))]
    async fn list_machines(
        &self,
        request: Request<ListMachinesRequest>,
    ) -> Result<Response<ListMachinesResponse>, Status> {
        let user_id = {
            let claims = extract_claims(&request)?;
            claims.sub.clone()
        };
        let req = request.into_inner();

        let status_filter = match MachineStatus::try_from(req.status_filter) {
            Ok(MachineStatus::Online) => Some("online"),
            Ok(MachineStatus::Offline) => Some("offline"),
            _ => None,
        };

        let limit = if req.limit == 0 { 100 } else { req.limit };

        let machines = self
            .db
            .list_machines(&user_id, status_filter, limit, req.offset)
            .await
            .map_err(|e| Status::internal(format!("Failed to list machines: {e}")))?;

        let total = self
            .db
            .count_machines(&user_id, status_filter)
            .await
            .map_err(|e| Status::internal(format!("Failed to count machines: {e}")))?;

        Ok(Response::new(ListMachinesResponse {
            machines: machines.iter().map(machine_to_proto).collect(),
            total: u32::try_from(total).unwrap_or(u32::MAX),
        }))
    }

    #[instrument(skip(self, request), fields(rpc = "RemoveMachine"))]
    async fn remove_machine(
        &self,
        request: Request<RemoveMachineRequest>,
    ) -> Result<Response<RemoveMachineResponse>, Status> {
        let user_id = {
            let claims = extract_claims(&request)?;
            claims.sub.clone()
        };
        let req = request.into_inner();

        // Verify ownership
        let machine = self
            .db
            .get_machine(&req.machine_id)
            .await
            .map_err(|_| Status::not_found("Machine not found"))?;

        if machine.owner_id != user_id {
            return Err(Status::permission_denied("Not your machine"));
        }

        let removed = self
            .db
            .remove_machine(&req.machine_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to remove machine: {e}")))?;

        info!(machine_id = %req.machine_id, "Machine removed");

        Ok(Response::new(RemoveMachineResponse { removed }))
    }

    #[instrument(skip(self, request), fields(rpc = "GetMachine"))]
    async fn get_machine(
        &self,
        request: Request<GetMachineRequest>,
    ) -> Result<Response<GetMachineResponse>, Status> {
        let user_id = {
            let claims = extract_claims(&request)?;
            claims.sub.clone()
        };
        let req = request.into_inner();

        let machine = self
            .db
            .get_machine(&req.machine_id)
            .await
            .map_err(|_| Status::not_found("Machine not found"))?;

        if machine.owner_id != user_id {
            return Err(Status::permission_denied("Not your machine"));
        }

        Ok(Response::new(GetMachineResponse {
            machine: Some(machine_to_proto(&machine)),
        }))
    }
}
