use axum::body::Body;
use axum::response::Response;

#[derive(Debug)]
pub struct Handoff {
    pub moved: bool,
    pub keep_alive: bool,
    pub response: Option<Response<Body>>,
}

impl Handoff {
    pub fn new(response: Response<Body>) -> Self {
        Self {
            moved: false,
            keep_alive: true,
            response: Some(response),
        }
    }

    pub fn moved() -> Self {
        Self {
            moved: true,
            keep_alive: true,
            response: None,
        }
    }
}

impl Default for Handoff {
    fn default() -> Self {
        Self {
            moved: false,
            keep_alive: true,
            response: None,
        }
    }
}
