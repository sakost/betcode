//! UserQuestion prompt input handling.

use std::collections::HashMap;

use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::app::{App, AppMode};
use betcode_proto::v1::{AgentRequest, UserQuestionResponse};

/// Handle a key press during a UserQuestion prompt.
pub async fn handle_question_key(
    app: &mut App,
    tx: &mpsc::Sender<AgentRequest>,
    code: KeyCode,
) {
    match code {
        KeyCode::Up => {
            app.move_question_highlight(-1);
        }
        KeyCode::Down => {
            app.move_question_highlight(1);
        }
        KeyCode::Char(' ') => {
            if let Some(ref q) = app.pending_question {
                let idx = q.highlight;
                app.select_question_option(idx);
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let index = (c as u8 - b'1') as usize;
            app.select_question_option(index);
        }
        KeyCode::Enter => {
            submit_question(app, tx).await;
        }
        KeyCode::Esc => {
            app.pending_question = None;
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
}

/// Submit the selected question answers.
async fn submit_question(app: &mut App, tx: &mpsc::Sender<AgentRequest>) {
    if let Some(ref q) = app.pending_question {
        if q.selected.is_empty() {
            return;
        }
        let mut answers = HashMap::new();
        let selected_labels: Vec<String> = q
            .selected
            .iter()
            .filter_map(|&i| q.options.get(i).map(|o| o.label.clone()))
            .collect();
        answers.insert(q.question.clone(), selected_labels.join(", "));

        let _ = tx
            .send(AgentRequest {
                request: Some(
                    betcode_proto::v1::agent_request::Request::QuestionResponse(
                        UserQuestionResponse {
                            question_id: q.question_id.clone(),
                            answers,
                        },
                    ),
                ),
            })
            .await;
    }
    app.pending_question = None;
    app.mode = AppMode::Normal;
}
