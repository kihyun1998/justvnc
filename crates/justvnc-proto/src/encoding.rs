//! Encoding and pseudo-encoding numbers, as sent in SetEncodings and rectangle headers.

pub const RAW: i32 = 0;
pub const COPY_RECT: i32 = 1;
pub const TIGHT: i32 = 7;
pub const ZRLE: i32 = 16;
pub const DESKTOP_SIZE: i32 = -223;
pub const LAST_RECT: i32 = -224;
