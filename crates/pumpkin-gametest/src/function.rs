use std::sync::Mutex;

use crate::GameTestResult;

/// Supplies a function test body without blocking the game tick.
pub trait GameTestFunction: Send + Sync {
    fn start(&self, execution: std::sync::Arc<GameTestExecution>) -> GameTestResult<()>;
}

/// Completion shared between an asynchronous test body and the tick runner.
#[derive(Default)]
pub struct GameTestExecution {
    state: Mutex<ExecutionState>,
}

#[derive(Default)]
struct ExecutionState {
    result: Option<Result<(), String>>,
    closed: bool,
}

impl GameTestExecution {
    #[must_use]
    pub fn is_active(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        !state.closed && state.result.is_none()
    }

    pub fn succeed(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.result.is_some() {
            return Err("GameTest has already completed".to_owned());
        }
        state.result = Some(Ok(()));
        Ok(())
    }

    pub fn fail(&self, message: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.closed && state.result.is_none() {
            state.result = Some(Err(message));
        }
    }

    pub(crate) fn result(&self) -> Option<Result<(), String>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .result
            .clone()
    }

    pub(crate) fn close(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .closed = true;
    }
}
