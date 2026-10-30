//! The mouse pointer shapes the client draws: sway reports the name of the shape the application asked for (`text`, `ew-resize`, ...),
//! and the matching small image goes to the client as an RDP pointer, so the pointer itself moves without any delay and only its
//! shape arrives a round trip late. The images are made here (white with a black outline, 32 x 32), there is no cursor theme to read.

use ironrdp_server::RGBAPointer;

const SIZE: usize = 32;

/// The shapes there are, in the order of their pointer cache index (1 to 19; 0 is the empty pointer).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Arrow,
    Text,
    Hand,
    Wait,
    Cross,
    Move,
    ResizeH,
    ResizeV,
    ResizeNwse,
    ResizeNesw,
    NotAllowed,
    Grab,
    Hidden,
}

/// The shape for a cursor name (the CSS names of cursor-shape-v1, and the older X names), the arrow for any other.
fn shape_of(name: &str) -> Shape {
    match name {
        "text" | "vertical-text" | "xterm" | "ibeam" => Shape::Text,
        "pointer" | "alias" | "hand1" | "hand2" => Shape::Hand,
        "wait" | "progress" | "watch" => Shape::Wait,
        "crosshair" | "cell" | "cross" | "tcross" | "zoom-in" | "zoom-out" => Shape::Cross,
        "move" | "all-scroll" | "fleur" | "size_all" => Shape::Move,
        "ew-resize" | "col-resize" | "e-resize" | "w-resize" | "sb_h_double_arrow" | "h_double_arrow" | "split_h" => Shape::ResizeH,
        "ns-resize" | "row-resize" | "n-resize" | "s-resize" | "sb_v_double_arrow" | "v_double_arrow" | "split_v" => Shape::ResizeV,
        "nwse-resize" | "nw-resize" | "se-resize" | "top_left_corner" | "bottom_right_corner" => Shape::ResizeNwse,
        "nesw-resize" | "ne-resize" | "sw-resize" | "top_right_corner" | "bottom_left_corner" => Shape::ResizeNesw,
        "not-allowed" | "no-drop" | "crossed_circle" => Shape::NotAllowed,
        "grab" | "grabbing" => Shape::Grab,
        "none" => Shape::Hidden,
        _ => Shape::Arrow,
    }
}

/// Whether the sway of the session reports the pointer (waydesk-sway does, a distribution's own sway does not). Then the compositor draws
/// no pointer, the apps may use a real cursor theme, and the pointer the client draws follows what the apps ask for.
pub fn sway_reports_cursor() -> bool {
    crate::compositor::kind() == crate::compositor::Kind::Sway && std::path::Path::new("/usr/lib/waydesk/bin/sway").is_file()
}

/// A pointer picture from the session is untrusted: a small one, of the size it says.
pub fn valid_image(width: u16, height: u16, len: usize) -> bool {
    (1..=32).contains(&width) && (1..=32).contains(&height) && len == usize::from(width) * usize::from(height) * 4
}

/// The pointer for a picture an application drew itself: 32 bit BGRA, premultiplied, rows from the top; made into a 32 x 32 pointer
/// image (straight alpha, rows from the bottom up).
pub fn pointer_from_image(hot_x: u16, hot_y: u16, width: u16, height: u16, data: &[u8]) -> RGBAPointer {
    let (w, h) = (usize::from(width).min(SIZE), usize::from(height).min(SIZE));
    let stride = usize::from(width);
    let mut out = vec![0u8; SIZE * SIZE * 4];
    for y in 0..h {
        for x in 0..w {
            let src = (y * stride + x) * 4;
            let Some(px) = data.get(src..src + 4) else { continue };
            let a = u32::from(px[3]);
            if a == 0 {
                continue;
            }
            let straight = |c: u8| if a == 255 { c } else { ((u32::from(c) * 255 + a / 2) / a).min(255) as u8 };
            let dst = ((SIZE - 1 - y) * SIZE + x) * 4;
            out[dst..dst + 4].copy_from_slice(&[straight(px[0]), straight(px[1]), straight(px[2]), px[3]]);
        }
    }
    RGBAPointer { cache_index: 15, width: SIZE as u16, height: SIZE as u16, hot_x: hot_x.min(SIZE as u16 - 1), hot_y: hot_y.min(SIZE as u16 - 1), data: out }
}

/// The message for a tick event payload of the private sway: `cursor:NAME` or `cursorimg:HOTX,HOTY,W,H,BASE64`, None for anything else.
pub fn message_from_tick(payload: &str) -> Option<crate::proto::Up> {
    use crate::proto::Up;
    if let Some(name) = payload.strip_prefix("cursor:") {
        return Some(Up::Cursor { name: name.to_string() });
    }
    let mut parts = payload.strip_prefix("cursorimg:")?.splitn(5, ',');
    let (hot_x, hot_y, width, height) = (
        parts.next()?.parse::<i32>().ok()?,
        parts.next()?.parse::<i32>().ok()?,
        parts.next()?.parse::<u16>().ok()?,
        parts.next()?.parse::<u16>().ok()?,
    );
    let data = crate::base64::decode(parts.next()?)?;
    if width > 64 || height > 64 || data.len() != usize::from(width) * usize::from(height) * 4 {
        return None;
    }
    Some(Up::CursorImage { hot_x: hot_x.clamp(0, 63) as u16, hot_y: hot_y.clamp(0, 63) as u16, width, height, data })
}

/// A cursor name from the session is untrusted: short, and letters, digits, `-` and `_` only.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// The pointer image for a cursor name.
pub fn pointer(name: &str) -> RGBAPointer {
    let shape = shape_of(name);
    let (mask, hot) = draw(shape);
    RGBAPointer { cache_index: shape as u16 + 1, width: SIZE as u16, height: SIZE as u16, hot_x: hot.0, hot_y: hot.1, data: to_rgba(&mask) }
}

type Mask = [[bool; SIZE]; SIZE];

fn draw(shape: Shape) -> (Mask, (u16, u16)) {
    let mut m: Mask = [[false; SIZE]; SIZE];
    // A point of the shape in pixel coordinates (x right, y down).
    let mut put = |x: i32, y: i32| {
        if (0..SIZE as i32).contains(&x) && (0..SIZE as i32).contains(&y) {
            m[y as usize][x as usize] = true;
        }
    };
    let c = 15.5f32;
    let hot = match shape {
        Shape::Arrow => {
            // The classic arrow, one row per line.
            const ART: [&str; 19] = [
                "X", "XX", "XXX", "XXXX", "XXXXX", "XXXXXX", "XXXXXXX", "XXXXXXXX", "XXXXXXXXX", "XXXXXXXXXX", "XXXXXXXXXXX", "XXXXXXX", "XXX XXXX",
                "XX  XXXX", "X    XXXX", "     XXXX", "      XXXX", "      XXXX", "       XX",
            ];
            for (y, line) in ART.iter().enumerate() {
                for (x, ch) in line.chars().enumerate() {
                    if ch == 'X' {
                        put(x as i32 + 1, y as i32 + 1);
                    }
                }
            }
            (1, 1)
        }
        Shape::Hidden => (0, 0),
        Shape::Text => {
            for y in 6..26 {
                put(15, y);
                put(16, y);
            }
            for x in 11..21 {
                for y in [6, 7, 24, 25] {
                    put(x, y);
                }
            }
            (16, 16)
        }
        Shape::Hand => {
            // The index finger up, the palm and three short fingers.
            for y in 3..14 {
                for x in 10..13 {
                    put(x, y);
                }
            }
            for y in 11..14 {
                for x in 13..22 {
                    put(x, y);
                }
            }
            for y in 13..25 {
                for x in 8..22 {
                    put(x, y);
                }
            }
            for y in 14..24 {
                put(7, y);
                put(22, y);
            }
            (11, 3)
        }
        Shape::Wait => {
            // An hourglass.
            for y in 5..27i32 {
                let half = if y < 16 { 9 - (y - 5) * 8 / 11 } else { 1 + (y - 16) * 8 / 10 };
                for x in (16 - half)..(16 + half) {
                    put(x, y);
                }
            }
            for x in 6..26 {
                for y in [3, 4, 27, 28] {
                    put(x, y);
                }
            }
            (16, 16)
        }
        Shape::Cross => {
            for i in 3..29 {
                if !(13..19).contains(&i) {
                    put(15, i);
                    put(16, i);
                    put(i, 15);
                    put(i, 16);
                }
            }
            (16, 16)
        }
        Shape::NotAllowed => {
            for y in 0..SIZE as i32 {
                for x in 0..SIZE as i32 {
                    let (dx, dy) = (x as f32 - c, y as f32 - c);
                    let r = (dx * dx + dy * dy).sqrt();
                    let ring = (8.0..=11.5).contains(&r);
                    let slash = (dx + dy).abs() < 2.2 && r < 11.0;
                    if ring || slash {
                        put(x, y);
                    }
                }
            }
            (16, 16)
        }
        Shape::Grab => {
            // A closed hand: a rounded block with the knuckles on top.
            for y in 10..25 {
                for x in 8..24 {
                    put(x, y);
                }
            }
            for k in 0..4 {
                for y in 7..11 {
                    for x in (9 + k * 4)..(12 + k * 4) {
                        put(x, y);
                    }
                }
            }
            (16, 16)
        }
        Shape::Move | Shape::ResizeH | Shape::ResizeV | Shape::ResizeNwse | Shape::ResizeNesw => {
            let dirs: &[(f32, f32)] = match shape {
                Shape::Move => &[(1.0, 0.0), (0.0, 1.0)],
                Shape::ResizeH => &[(1.0, 0.0)],
                Shape::ResizeV => &[(0.0, 1.0)],
                Shape::ResizeNwse => &[(0.7071, 0.7071)],
                _ => &[(0.7071, -0.7071)],
            };
            for y in 0..SIZE as i32 {
                for x in 0..SIZE as i32 {
                    let (px, py) = (x as f32 - c, y as f32 - c);
                    for &(dx, dy) in dirs {
                        let along = (px * dx + py * dy).abs();
                        let across = (px * dy - py * dx).abs();
                        let shaft = across <= 1.4 && along <= 11.0;
                        let head = along >= 6.0 && along <= 13.0 && across <= (13.0 - along) * 0.85;
                        if shaft || head {
                            put(x, y);
                        }
                    }
                }
            }
            (16, 16)
        }
    };
    (m, hot)
}

/// White where the shape is, black around it, the rest transparent: the rows of an RDP pointer run from the bottom up and the pixels
/// are blue, green, red, alpha.
fn to_rgba(mask: &Mask) -> Vec<u8> {
    let mut data = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let near = |dx: i32, dy: i32| {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                (0..SIZE as i32).contains(&nx) && (0..SIZE as i32).contains(&ny) && mask[ny as usize][nx as usize]
            };
            let value = if mask[y][x] {
                255
            } else if (-1..=1).any(|dy| (-1..=1).any(|dx| near(dx, dy))) {
                0
            } else {
                continue;
            };
            let at = ((SIZE - 1 - y) * SIZE + x) * 4;
            data[at..at + 4].copy_from_slice(&[value, value, value, 255]);
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_shapes() {
        assert_eq!(shape_of("text"), Shape::Text);
        assert_eq!(shape_of("ew-resize"), Shape::ResizeH);
        assert_eq!(shape_of("left_ptr"), Shape::Arrow);
        assert_eq!(shape_of("something-new"), Shape::Arrow);
    }

    #[test]
    fn none_is_a_pointer_with_nothing_in_it() {
        let p = pointer("none");
        assert!(p.data.iter().all(|&b| b == 0));
        assert_eq!(p.data.len(), 32 * 32 * 4);
    }

    #[test]
    fn pictures_become_pointers() {
        // a 2 x 1 picture: an opaque red pixel and half transparent white one (premultiplied: 128, 128, 128, 128)
        let data = [0, 0, 255, 255, 128, 128, 128, 128];
        let p = pointer_from_image(1, 0, 2, 1, &data);
        let last_row = 31 * 32 * 4; // the top row of the picture is the last row of the pointer
        assert_eq!(&p.data[last_row..last_row + 4], &[0, 0, 255, 255]);
        assert_eq!(&p.data[last_row + 4..last_row + 8], &[255, 255, 255, 128]);
        assert_eq!((p.hot_x, p.hot_y, p.width, p.height), (1, 0, 32, 32));
        assert!(valid_image(2, 1, 8) && !valid_image(2, 1, 7) && !valid_image(33, 1, 132) && !valid_image(0, 1, 0));
    }

    #[test]
    fn tick_payloads_become_messages() {
        use crate::proto::Up;
        assert_eq!(message_from_tick("cursor:text"), Some(Up::Cursor { name: "text".into() }));
        // base64 of the eight bytes 0 1 2 3 4 5 6 7
        let up = message_from_tick("cursorimg:3,4,2,1,AAECAwQFBgc=");
        assert_eq!(up, Some(Up::CursorImage { hot_x: 3, hot_y: 4, width: 2, height: 1, data: vec![0, 1, 2, 3, 4, 5, 6, 7] }));
        assert_eq!(message_from_tick("cursorimg:3,4,2,2,AAECAwQFBgc="), None); // wrong size
        assert_eq!(message_from_tick("cursorimg:3,4,2,1,@@@@"), None);
        assert_eq!(message_from_tick("something"), None);
    }

    #[test]
    fn names_from_the_session_are_checked() {
        assert!(valid_name("ns-resize"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name(&"x".repeat(41)));
    }

    #[test]
    fn every_shape_has_pixels_and_a_hot_spot_inside() {
        for name in ["default", "text", "pointer", "wait", "crosshair", "move", "ew-resize", "ns-resize", "nwse-resize", "nesw-resize", "not-allowed", "grab"] {
            let p = pointer(name);
            assert!(p.data.chunks(4).any(|px| px[3] == 255), "{name} is empty");
            assert!(p.hot_x < 32 && p.hot_y < 32);
            assert_eq!(p.data.len(), 32 * 32 * 4);
        }
    }
}
