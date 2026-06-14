pub mod atx;
pub mod dc;

pub use atx::{
    ATX_EXTENSION_ID, AtxState, get_atx_state, mount_atx_control, press_atx_power_button,
    press_atx_reset_button, set_atx_power_action, unmount_atx_control,
};
pub use dc::{
    DcPowerState, get_active_extension, get_dc_power_state, get_dc_restore_state, mount_dc_control,
    set_active_extension, set_dc_power_state, set_dc_restore_state, unmount_dc_control,
};
