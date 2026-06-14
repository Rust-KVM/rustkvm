use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdnsListenOptions {
    pub ipv4: bool,
    pub ipv6: bool,
}

impl Default for MdnsListenOptions {
    fn default() -> Self {
        Self { ipv4: true, ipv6: true }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MdnsOptions {
    pub local_names: Vec<String>,
    pub listen_options: MdnsListenOptions,
}

impl MdnsOptions {
    pub fn new(local_names: Vec<String>, listen_options: MdnsListenOptions) -> Self {
        Self { local_names, listen_options }
    }
}
