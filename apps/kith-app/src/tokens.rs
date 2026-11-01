//! The few design tokens the shell paints with natively, before or outside the page: the
//! window's background while the page loads, and the tray icon where the OS does not
//! recolour it. Each mirrors `tokens.css`; a test keeps them equal.

/// An opaque colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rgb(pub(crate) u8, pub(crate) u8, pub(crate) u8);

/// `--bg`, light.
pub(crate) const BG_LIGHT: Rgb = Rgb(0xF9, 0xFA, 0xFB);
/// `--bg`, dark.
pub(crate) const BG_DARK: Rgb = Rgb(0x11, 0x13, 0x16);
/// `--ink-1`, dark: the tray icon on panels that do not recolour icons, which are dark.
pub(crate) const INK_ON_PANEL: Rgb = Rgb(0xED, 0xEE, 0xF0);
/// `--attn`, dark: the needs-you dot on those panels.
pub(crate) const ATTN_ON_PANEL: Rgb = Rgb(0xE2, 0xA8, 0x4A);

#[cfg(test)]
mod tests {
    use super::*;

    const TOKENS: &str = include_str!("../../../crates/management/kith-ui/web/src/lib/tokens.css");

    fn hex(rgb: Rgb) -> String {
        format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2)
    }

    /// The value of `name` in the block that starts with `selector`.
    fn token(selector: &str, name: &str) -> Option<String> {
        let block = TOKENS.split_once(selector)?.1.split_once('}')?.0;
        block.lines().find_map(|line| {
            let value = line.trim().strip_prefix(name)?.strip_prefix(':')?;
            Some(value.trim().trim_end_matches(';').to_lowercase())
        })
    }

    #[test]
    fn native_colours_match_the_tokens() {
        let light = ":root {";
        let dark = ":root[data-theme='dark'] {";
        assert_eq!(token(light, "--bg"), Some(hex(BG_LIGHT)));
        assert_eq!(token(dark, "--bg"), Some(hex(BG_DARK)));
        assert_eq!(token(dark, "--ink-1"), Some(hex(INK_ON_PANEL)));
        assert_eq!(token(dark, "--attn"), Some(hex(ATTN_ON_PANEL)));
    }
}
