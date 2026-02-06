//! Tests for permission input handling.

#[cfg(test)]
mod tests {
    use crate::app::{App, AppMode, PendingPermission};
    use betcode_proto::v1::{AgentRequest, PermissionDecision};
    use crossterm::event::KeyCode;
    use tokio::sync::mpsc;

    fn make_app() -> App {
        let mut app = App::new();
        app.mode = AppMode::PermissionPrompt;
        app.pending_permission = Some(PendingPermission {
            request_id: "req-1".to_string(),
            tool_name: "Bash".to_string(),
            description: "ls -la".to_string(),
            original_input: Some(serde_json::json!({"command": "ls -la"})),
            edit_buffer: String::new(),
            edit_cursor: 0,
            deny_interrupt: false,
        });
        app
    }

    fn extract_decision(req: &AgentRequest) -> (i32, String) {
        match &req.request {
            Some(betcode_proto::v1::agent_request::Request::Permission(p)) => {
                (p.decision, p.message.clone())
            }
            _ => panic!("Expected permission response"),
        }
    }

    // -- Permission prompt keys --

    #[tokio::test]
    async fn y_sends_allow_once() {
        let mut app = make_app();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('y')).await;
        let (d, _) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::AllowOnce as i32);
        assert!(app.pending_permission.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[tokio::test]
    async fn uppercase_y_sends_allow_once() {
        let mut app = make_app();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('Y')).await;
        let (d, _) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::AllowOnce as i32);
    }

    #[tokio::test]
    async fn a_sends_allow_session() {
        let mut app = make_app();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('a')).await;
        let (d, _) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::AllowSession as i32);
    }

    #[tokio::test]
    async fn tab_enters_edit_mode() {
        let mut app = make_app();
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Tab).await;
        assert_eq!(app.mode, AppMode::PermissionEditInput);
        assert!(!app.pending_permission.as_ref().unwrap().edit_buffer.is_empty());
    }

    #[tokio::test]
    async fn e_enters_comment_mode() {
        let mut app = make_app();
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('e')).await;
        assert_eq!(app.mode, AppMode::PermissionComment);
        assert!(app.pending_permission.as_ref().unwrap().edit_buffer.is_empty());
    }

    #[tokio::test]
    async fn n_enters_deny_no_interrupt() {
        let mut app = make_app();
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('n')).await;
        assert_eq!(app.mode, AppMode::PermissionDenyMessage);
        assert!(!app.pending_permission.as_ref().unwrap().deny_interrupt);
    }

    #[tokio::test]
    async fn x_enters_deny_with_interrupt() {
        let mut app = make_app();
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('x')).await;
        assert_eq!(app.mode, AppMode::PermissionDenyMessage);
        assert!(app.pending_permission.as_ref().unwrap().deny_interrupt);
    }

    #[tokio::test]
    async fn esc_sends_deny() {
        let mut app = make_app();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Esc).await;
        let (d, _) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::Deny as i32);
        assert!(app.pending_permission.is_none());
    }

    #[tokio::test]
    async fn number_1_sends_allow_once() {
        let mut app = make_app();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('1')).await;
        let (d, _) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::AllowOnce as i32);
    }

    #[tokio::test]
    async fn number_2_sends_allow_session() {
        let mut app = make_app();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('2')).await;
        let (d, _) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::AllowSession as i32);
    }

    #[tokio::test]
    async fn number_3_enters_edit_mode() {
        let mut app = make_app();
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('3')).await;
        assert_eq!(app.mode, AppMode::PermissionEditInput);
    }

    #[tokio::test]
    async fn number_5_enters_deny_no_interrupt() {
        let mut app = make_app();
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('5')).await;
        assert_eq!(app.mode, AppMode::PermissionDenyMessage);
        assert!(!app.pending_permission.as_ref().unwrap().deny_interrupt);
    }

    #[tokio::test]
    async fn number_6_enters_deny_with_interrupt() {
        let mut app = make_app();
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_key(&mut app, &tx, KeyCode::Char('6')).await;
        assert_eq!(app.mode, AppMode::PermissionDenyMessage);
        assert!(app.pending_permission.as_ref().unwrap().deny_interrupt);
    }

    // -- Permission edit keys --

    #[tokio::test]
    async fn edit_typing_inserts_chars() {
        let mut app = make_app();
        app.mode = AppMode::PermissionComment;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        for c in "hello".chars() {
            crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Char(c))
                .await;
        }
        let perm = app.pending_permission.as_ref().unwrap();
        assert_eq!(perm.edit_buffer, "hello");
        assert_eq!(perm.edit_cursor, 5);
    }

    #[tokio::test]
    async fn edit_backspace_removes_char() {
        let mut app = make_app();
        app.mode = AppMode::PermissionComment;
        app.pending_permission.as_mut().unwrap().edit_buffer = "abc".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 3;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Backspace)
            .await;
        let perm = app.pending_permission.as_ref().unwrap();
        assert_eq!(perm.edit_buffer, "ab");
        assert_eq!(perm.edit_cursor, 2);
    }

    #[tokio::test]
    async fn edit_esc_returns_to_prompt() {
        let mut app = make_app();
        app.mode = AppMode::PermissionComment;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Esc).await;
        assert_eq!(app.mode, AppMode::PermissionPrompt);
        assert!(app.pending_permission.is_some());
    }

    #[tokio::test]
    async fn comment_enter_sends_allow_and_message() {
        let mut app = make_app();
        app.mode = AppMode::PermissionComment;
        app.pending_permission.as_mut().unwrap().edit_buffer = "use caution".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 11;
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Enter).await;

        let (d, _) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::AllowOnce as i32);
        // Follow-up comment message
        match &rx.try_recv().unwrap().request {
            Some(betcode_proto::v1::agent_request::Request::Message(m)) => {
                assert_eq!(m.content, "use caution");
            }
            _ => panic!("Expected user message"),
        }
        assert!(app.pending_permission.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[tokio::test]
    async fn deny_enter_sends_deny_no_interrupt_with_message() {
        let mut app = make_app();
        app.mode = AppMode::PermissionDenyMessage;
        app.pending_permission.as_mut().unwrap().deny_interrupt = false;
        app.pending_permission.as_mut().unwrap().edit_buffer = "not allowed".to_string();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Enter).await;
        let (d, msg) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::DenyNoInterrupt as i32);
        assert_eq!(msg, "not allowed");
    }

    #[tokio::test]
    async fn deny_enter_sends_deny_with_interrupt_and_message() {
        let mut app = make_app();
        app.mode = AppMode::PermissionDenyMessage;
        app.pending_permission.as_mut().unwrap().deny_interrupt = true;
        app.pending_permission.as_mut().unwrap().edit_buffer = "stop now".to_string();
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Enter).await;
        let (d, msg) = extract_decision(&rx.try_recv().unwrap());
        assert_eq!(d, PermissionDecision::DenyWithInterrupt as i32);
        assert_eq!(msg, "stop now");
    }

    #[tokio::test]
    async fn edit_left_right_moves_cursor() {
        let mut app = make_app();
        app.mode = AppMode::PermissionComment;
        app.pending_permission.as_mut().unwrap().edit_buffer = "abc".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 3;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Left).await;
        assert_eq!(app.pending_permission.as_ref().unwrap().edit_cursor, 2);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Right).await;
        assert_eq!(app.pending_permission.as_ref().unwrap().edit_cursor, 3);
    }

    #[tokio::test]
    async fn edit_left_clamps_at_zero() {
        let mut app = make_app();
        app.mode = AppMode::PermissionComment;
        app.pending_permission.as_mut().unwrap().edit_cursor = 0;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Left).await;
        assert_eq!(app.pending_permission.as_ref().unwrap().edit_cursor, 0);
    }

    #[tokio::test]
    async fn edit_right_clamps_at_end() {
        let mut app = make_app();
        app.mode = AppMode::PermissionComment;
        app.pending_permission.as_mut().unwrap().edit_buffer = "ab".to_string();
        app.pending_permission.as_mut().unwrap().edit_cursor = 2;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::permission_input::handle_permission_edit_key(&mut app, &tx, KeyCode::Right).await;
        assert_eq!(app.pending_permission.as_ref().unwrap().edit_cursor, 2);
    }
}
