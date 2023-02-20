use std::{collections::HashMap, sync::Arc};

use crate::server::Server;

#[derive(Default)]
pub struct Router {
    pub data: HashMap<Option<String>, Arc<Server>>,
}

impl Router {
    pub fn get(&self, server_name: String) -> Option<Arc<Server>> {
        self.data
            .get(&Some(server_name))
            .or_else(|| self.data.get(&None))
            .cloned()
    }
}
