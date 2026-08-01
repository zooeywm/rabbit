mod inbound;

use crate::domain::stream::models::entity::Stream;

pub struct Container {
    stream: Vec<Stream>,
}

impl Container {
    pub fn new() -> Self {
        Self { stream: Vec::new() }
    }
}
