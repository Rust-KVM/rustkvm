use rustkvm::hardware::hw::summarize_kernel_crash;
use rustkvm::hardware::usb::udc_link_up;

// The HID files must be closed whenever the kernel may rebind the gadget, which
// is after the host detaches (Type-C role switch), never while a link exists.
#[test]
fn hid_files_close_only_without_a_host_link() {
    for state in ["not attached", "unknown", ""] {
        assert!(!udc_link_up(state), "{state:?} must close the HID files");
    }
    for state in ["attached", "powered", "default", "addressed", "configured", "suspended"] {
        assert!(udc_link_up(state), "{state:?} must keep the HID files open");
    }
}

#[test]
fn kernel_crash_summary_keeps_the_identifying_lines() {
    let log = "\
<6>[160067.590123] xhci-hcd xhci-hcd.15.auto: remove, state 4
<3>[160067.715344] list_del corruption. prev->next should be ffffffc00b03bb70, but was ffffff81044e4048
<0>[160067.716000] Internal error: Oops - BUG: 00000000f2000800 [#1] SMP
<4>[160067.716100] CPU: 0 PID: 610 Comm: hid-kbd-reader Tainted: G           O       6.1.99 #4
<4>[160067.716200] pc : __list_del_entry_valid+0x9c/0xa0
<4>[160067.716300] lr : __list_del_entry_valid+0x9c/0xa0
<4>[160067.716400] x29: ffffffc00b03bb30 x28: 0000000000000000
";
    let summary = summarize_kernel_crash(log);
    assert_eq!(summary.len(), 5);
    assert!(summary[0].contains("list_del corruption"));
    assert!(summary[2].contains("Comm: hid-kbd-reader"));
    assert!(summarize_kernel_crash("<6>[1.0] normal boot\n").is_empty());
}
