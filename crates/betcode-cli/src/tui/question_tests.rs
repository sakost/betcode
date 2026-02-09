//! Tests for user question input handling.

#[cfg(test)]
mod tests {
    use crate::app::{App, AppMode, PendingUserQuestion, QuestionOptionDisplay};
    use betcode_proto::v1::AgentRequest;
    use crossterm::event::KeyCode;
    use tokio::sync::mpsc;

    fn make_app(multi_select: bool) -> App {
        let mut app = App::new();
        app.mode = AppMode::UserQuestion;
        app.pending_question = Some(PendingUserQuestion {
            question_id: "q-1".to_string(),
            question: "Which option?".to_string(),
            options: vec![
                QuestionOptionDisplay {
                    label: "Alpha".to_string(),
                    description: "First".to_string(),
                },
                QuestionOptionDisplay {
                    label: "Beta".to_string(),
                    description: "Second".to_string(),
                },
                QuestionOptionDisplay {
                    label: "Gamma".to_string(),
                    description: "Third".to_string(),
                },
            ],
            multi_select,
            highlight: 0,
            selected: Vec::new(),
        });
        app
    }

    fn extract_question(req: &AgentRequest) -> (String, std::collections::HashMap<String, String>) {
        match &req.request {
            Some(betcode_proto::v1::agent_request::Request::QuestionResponse(q)) => {
                (q.question_id.clone(), q.answers.clone())
            }
            _ => panic!("Expected question response"),
        }
    }

    // -- Navigation --

    #[tokio::test]
    async fn arrow_down_moves_highlight() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Down).await;
        assert_eq!(app.pending_question.as_ref().unwrap().highlight, 1);
    }

    #[tokio::test]
    async fn arrow_up_moves_highlight() {
        let mut app = make_app(false);
        app.pending_question.as_mut().unwrap().highlight = 2;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Up).await;
        assert_eq!(app.pending_question.as_ref().unwrap().highlight, 1);
    }

    #[tokio::test]
    async fn arrow_up_clamps_at_zero() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Up).await;
        assert_eq!(app.pending_question.as_ref().unwrap().highlight, 0);
    }

    #[tokio::test]
    async fn arrow_down_clamps_at_max() {
        let mut app = make_app(false);
        app.pending_question.as_mut().unwrap().highlight = 2;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Down).await;
        assert_eq!(app.pending_question.as_ref().unwrap().highlight, 2);
    }

    // -- Selection --

    #[tokio::test]
    async fn space_selects_highlighted_option() {
        let mut app = make_app(true);
        app.pending_question.as_mut().unwrap().highlight = 1;
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char(' ')).await;
        assert_eq!(app.pending_question.as_ref().unwrap().selected, vec![1]);
    }

    #[tokio::test]
    async fn space_toggles_multi_select() {
        let mut app = make_app(true);
        app.pending_question.as_mut().unwrap().highlight = 0;
        app.pending_question.as_mut().unwrap().selected = vec![0];
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char(' ')).await;
        assert!(app.pending_question.as_ref().unwrap().selected.is_empty());
    }

    #[tokio::test]
    async fn number_key_selects_option() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('2')).await;
        let q = app.pending_question.as_ref().unwrap();
        assert_eq!(q.highlight, 1);
        assert_eq!(q.selected, vec![1]);
    }

    #[tokio::test]
    async fn number_key_3_selects_third() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('3')).await;
        let q = app.pending_question.as_ref().unwrap();
        assert_eq!(q.highlight, 2);
        assert_eq!(q.selected, vec![2]);
    }

    #[tokio::test]
    async fn invalid_number_key_ignored() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        // '9' is out of bounds (only 3 options)
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('9')).await;
        assert!(app.pending_question.as_ref().unwrap().selected.is_empty());
    }

    #[tokio::test]
    async fn zero_key_ignored() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('0')).await;
        assert!(app.pending_question.as_ref().unwrap().selected.is_empty());
    }

    // -- Submission --

    #[tokio::test]
    async fn enter_submits_single_selection() {
        let mut app = make_app(false);
        app.pending_question.as_mut().unwrap().selected = vec![1];
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Enter).await;
        let (qid, answers) = extract_question(&rx.try_recv().unwrap());
        assert_eq!(qid, "q-1");
        assert_eq!(answers.get("Which option?").unwrap(), "Beta");
        assert!(app.pending_question.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[tokio::test]
    async fn enter_submits_multi_selection_joined() {
        let mut app = make_app(true);
        app.pending_question.as_mut().unwrap().selected = vec![0, 2];
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Enter).await;
        let (_, answers) = extract_question(&rx.try_recv().unwrap());
        assert_eq!(answers.get("Which option?").unwrap(), "Alpha, Gamma");
    }

    #[tokio::test]
    async fn enter_empty_selection_does_nothing() {
        let mut app = make_app(false);
        let (tx, mut rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Enter).await;
        assert!(rx.try_recv().is_err());
        assert!(app.pending_question.is_some());
    }

    // -- Cancellation --

    #[tokio::test]
    async fn esc_cancels_question() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Esc).await;
        assert!(app.pending_question.is_none());
        assert_eq!(app.mode, AppMode::Normal);
    }

    // -- Single select replaces selection --

    #[tokio::test]
    async fn single_select_replaces_previous() {
        let mut app = make_app(false);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('1')).await;
        assert_eq!(app.pending_question.as_ref().unwrap().selected, vec![0]);
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('3')).await;
        assert_eq!(app.pending_question.as_ref().unwrap().selected, vec![2]);
    }

    // -- Multi select accumulates --

    #[tokio::test]
    async fn multi_select_accumulates() {
        let mut app = make_app(true);
        let (tx, _rx) = mpsc::channel::<AgentRequest>(8);
        // Select via number keys
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('1')).await;
        crate::tui::question_input::handle_question_key(&mut app, &tx, KeyCode::Char('3')).await;
        let q = app.pending_question.as_ref().unwrap();
        assert!(q.selected.contains(&0));
        assert!(q.selected.contains(&2));
        assert_eq!(q.selected.len(), 2);
    }
}
