/// Upper bounds on lengths a peer announces, checked before anything is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest desktop name accepted in ServerInit, in bytes.
    pub max_name_length: u32,
    /// Longest failure reason accepted from a server, in bytes.
    pub max_reason_length: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_name_length: 4096,
            max_reason_length: 4096,
        }
    }
}
