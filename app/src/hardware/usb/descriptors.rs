//! USB HID report descriptors for keyboard and mouse devices.
//!
//! These descriptors define the structure of HID reports sent to and from
//! the USB HID devices. They are based on the USB HID specification.

/// Keyboard HID report descriptor
///
/// Source: USB HID specification https://www.kernel.org/doc/Documentation/usb/gadget_hid.txt
/// - USAGE_PAGE (Generic Desktop)
/// - USAGE (Keyboard)
/// - 8 modifier keys + 101 regular keys + 5 LED outputs
pub const KEYBOARD_REPORT_DESC: &[u8] = &[
    0x05, 0x01, /* USAGE_PAGE (Generic Desktop)	          */
    0x09, 0x06, /* USAGE (Keyboard)                       */
    0xa1, 0x01, /* COLLECTION (Application)               */
    0x05, 0x07, /*   USAGE_PAGE (Keyboard)                */
    0x19, 0xe0, /*   USAGE_MINIMUM (Keyboard LeftControl) */
    0x29, 0xe7, /*   USAGE_MAXIMUM (Keyboard Right GUI)   */
    0x15, 0x00, /*   LOGICAL_MINIMUM (0)                  */
    0x25, 0x01, /*   LOGICAL_MAXIMUM (1)                  */
    0x75, 0x01, /*   REPORT_SIZE (1)                      */
    0x95, 0x08, /*   REPORT_COUNT (8)                     */
    0x81, 0x02, /*   INPUT (Data,Var,Abs)                 */
    0x95, 0x01, /*   REPORT_COUNT (1)                     */
    0x75, 0x08, /*   REPORT_SIZE (8)                      */
    0x81, 0x03, /*   INPUT (Cnst,Var,Abs)                 */
    0x95, 0x05, /*   REPORT_COUNT (5)                     */
    0x75, 0x01, /*   REPORT_SIZE (1)                      */
    0x05, 0x08, /*   USAGE_PAGE (LEDs)                    */
    0x19, 0x01, /*   USAGE_MINIMUM (Num Lock)             */
    0x29, 0x05, /*   USAGE_MAXIMUM (Kana)                 */
    0x91, 0x02, /*   OUTPUT (Data,Var,Abs)                */
    0x95, 0x01, /*   REPORT_COUNT (1)                     */
    0x75, 0x03, /*   REPORT_SIZE (3)                      */
    0x91, 0x03, /*   OUTPUT (Cnst,Var,Abs)                */
    0x95, 0x06, /*   REPORT_COUNT (6)                     */
    0x75, 0x08, /*   REPORT_SIZE (8)                      */
    0x15, 0x00, /*   LOGICAL_MINIMUM (0)                  */
    0x25, 0x65, /*   LOGICAL_MAXIMUM (101)                */
    0x05, 0x07, /*   USAGE_PAGE (Keyboard)                */
    0x19, 0x00, /*   USAGE_MINIMUM (Reserved)             */
    0x29, 0x65, /*   USAGE_MAXIMUM (Keyboard Application) */
    0x81, 0x00, /*   INPUT (Data,Ary,Abs)                 */
    0xc0, /* END_COLLECTION                         */
];

/// Absolute mouse HID report descriptor with wheel support
///
/// Source: USB HID specification
/// - Report ID 1: Absolute mouse movement (X, Y, buttons)
/// - Report ID 2: Wheel movement
pub const ABSOLUTE_MOUSE_REPORT_DESC: &[u8] = &[
    0x05, 0x01, // Usage Page (Generic Desktop Ctrls)
    0x09, 0x02, // Usage (Mouse)
    0xA1, 0x01, // Collection (Application)
    // Report ID 1: Absolute Mouse Movement
    0x85, 0x01, //     Report ID (1)
    0x09, 0x01, //     Usage (Pointer)
    0xA1, 0x00, //     Collection (Physical)
    0x05, 0x09, //         Usage Page (Button)
    0x19, 0x01, //         Usage Minimum (0x01)
    0x29, 0x03, //         Usage Maximum (0x03)
    0x15, 0x00, //         Logical Minimum (0)
    0x25, 0x01, //         Logical Maximum (1)
    0x75, 0x01, //         Report Size (1)
    0x95, 0x03, //         Report Count (3)
    0x81, 0x02, //         Input (Data, Var, Abs)
    0x95, 0x01, //         Report Count (1)
    0x75, 0x05, //         Report Size (5)
    0x81, 0x03, //         Input (Cnst, Var, Abs)
    0x05, 0x01, //         Usage Page (Generic Desktop Ctrls)
    0x09, 0x30, //         Usage (X)
    0x09, 0x31, //         Usage (Y)
    0x16, 0x00, 0x00, //         Logical Minimum (0)
    0x26, 0xFF, 0x7F, //         Logical Maximum (32767)
    0x36, 0x00, 0x00, //         Physical Minimum (0)
    0x46, 0xFF, 0x7F, //         Physical Maximum (32767)
    0x75, 0x10, //         Report Size (16)
    0x95, 0x02, //         Report Count (2)
    0x81, 0x02, //         Input (Data, Var, Abs)
    0xC0, //     End Collection
    // Report ID 2: Relative Wheel Movement
    0x85, 0x02, //     Report ID (2)
    0x09, 0x38, //     Usage (Wheel)
    0x15, 0x81, //     Logical Minimum (-127)
    0x25, 0x7F, //     Logical Maximum (127)
    0x35, 0x00, //     Physical Minimum (0) = Reset Physical Minimum
    0x45, 0x00, //     Physical Maximum (0) = Reset Physical Maximum
    0x75, 0x08, //     Report Size (8)
    0x95, 0x01, //     Report Count (1)
    0x81, 0x06, //     Input (Data, Var, Rel)
    0xC0, // End Collection
];

/// Relative mouse HID report descriptor
///
/// Source: https://github.com/NicoHood/HID/blob/b16be57caef4295c6cd382a7e4c64db5073647f7/src/SingleReport/BootMouse.cpp#L26
/// - 8 buttons + X, Y, Wheel movement
/// - Boot protocol compatible
pub const RELATIVE_MOUSE_REPORT_DESC: &[u8] = &[
    0x05, 0x01, // USAGE_PAGE (Generic Desktop)	  54
    0x09, 0x02, // USAGE (Mouse)
    0xa1, 0x01, // COLLECTION (Application)
    // Pointer and Physical are required by Apple Recovery
    0x09, 0x01, // USAGE (Pointer)
    0xa1, 0x00, // COLLECTION (Physical)
    // 8 Buttons
    0x05, 0x09, // USAGE_PAGE (Button)
    0x19, 0x01, // USAGE_MINIMUM (Button 1)
    0x29, 0x08, // USAGE_MAXIMUM (Button 8)
    0x15, 0x00, // LOGICAL_MINIMUM (0)
    0x25, 0x01, // LOGICAL_MAXIMUM (1)
    0x95, 0x08, // REPORT_COUNT (8)
    0x75, 0x01, // REPORT_SIZE (1)
    0x81, 0x02, // INPUT (Data,Var,Abs)
    // X, Y, Wheel
    0x05, 0x01, // USAGE_PAGE (Generic Desktop)
    0x09, 0x30, // USAGE (X)
    0x09, 0x31, // USAGE (Y)
    0x09, 0x38, // USAGE (Wheel)
    0x15, 0x81, // LOGICAL_MINIMUM (-127)
    0x25, 0x7f, // LOGICAL_MAXIMUM (127)
    0x75, 0x08, // REPORT_SIZE (8)
    0x95, 0x03, // REPORT_COUNT (3)
    0x81, 0x06, // INPUT (Data,Var,Rel)
    // End
    0xc0, //       End Collection (Physical)
    0xc0, //       End Collection
];

/// Mass storage gadget configuration
///
/// Basic mass storage device configuration for virtual media support
pub const MASS_STORAGE_CONFIG: &[u8] = &[
    // Basic mass storage configuration
    // This is minimal as the actual configuration is handled by the kernel
    // and the lun.0 subdirectory structure
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyboard_descriptor_length() {
        // Ensure descriptor has reasonable length
        assert!(KEYBOARD_REPORT_DESC.len() > 50);
        assert!(KEYBOARD_REPORT_DESC.len() < 200);
    }

    #[test]
    fn test_absolute_mouse_descriptor_length() {
        // Ensure descriptor has reasonable length
        assert!(ABSOLUTE_MOUSE_REPORT_DESC.len() > 30);
        assert!(ABSOLUTE_MOUSE_REPORT_DESC.len() < 150);
    }

    #[test]
    fn test_relative_mouse_descriptor_length() {
        // Ensure descriptor has reasonable length
        assert!(RELATIVE_MOUSE_REPORT_DESC.len() > 20);
        assert!(RELATIVE_MOUSE_REPORT_DESC.len() < 100);
    }
}
