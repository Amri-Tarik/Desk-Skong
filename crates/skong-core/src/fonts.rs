//! The two typefaces Desk-Skong ships (embedded; SIL Open Font License, see
//! `crates/skong-core/fonts/OFL.txt`). Not game art: Hollow Knight's titles are set in
//! Trajan Pro, which can't be shipped, so a freely licensed lookalike stands in.
//!
//! - [`HEADING`]: **Cinzel** Regular — classical engraved capitals for headings and the
//!   achievement banner's title.
//! - [`BODY`]: **Crimson Pro** Regular — a book serif that stays readable at 15–17 px.
//!
//! The panel hands the bytes to egui; the pet registers them privately with GDI
//! (`skong_win::text::register_font`) and selects them by [`HEADING_FACE`] / [`BODY_FACE`].

pub const HEADING: &[u8] = include_bytes!("../fonts/Cinzel-Regular.ttf");
pub const BODY: &[u8] = include_bytes!("../fonts/CrimsonPro-Regular.ttf");
/// The licence text shipped beside the binaries.
pub const LICENSE: &str = include_str!("../fonts/OFL.txt");

/// GDI face names (the fonts' family names).
pub const HEADING_FACE: &str = "Cinzel";
pub const BODY_FACE: &str = "Crimson Pro";

/// The family name (name ID 1, Windows Unicode record) of a TrueType / OpenType font.
pub fn family_name(font: &[u8]) -> Option<String> {
    let u16_at = |o: usize| font.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |o: usize| font.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let tables = u16_at(4)? as usize;
    let name = (0..tables).map(|i| 12 + 16 * i).find(|&rec| font.get(rec..rec + 4) == Some(b"name"))?;
    let base = u32_at(name + 8)? as usize;
    let (count, strings) = (u16_at(base + 2)? as usize, base + u16_at(base + 4)? as usize);
    (0..count).map(|i| base + 6 + 12 * i).find_map(|r| {
        let (platform, encoding, id) = (u16_at(r)?, u16_at(r + 2)?, u16_at(r + 6)?);
        if (platform, encoding, id) != (3, 1, 1) {
            return None;
        }
        let (len, off) = (u16_at(r + 8)? as usize, u16_at(r + 10)? as usize);
        let raw = font.get(strings + off..strings + off + len)?;
        let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_fonts_are_truetype_with_the_expected_family_names() {
        for (font, face) in [(HEADING, HEADING_FACE), (BODY, BODY_FACE)] {
            assert_eq!(&font[..4], &[0, 1, 0, 0], "{face}: a TrueType outline font");
            assert_eq!(family_name(font).as_deref(), Some(face));
        }
        assert!(LICENSE.contains("SIL OPEN FONT LICENSE Version 1.1") && LICENSE.contains("Cinzel") && LICENSE.contains("Crimson Pro"));
    }

    #[test]
    fn family_name_survives_garbage() {
        assert_eq!(family_name(&[]), None);
        assert_eq!(family_name(&[0, 1, 0, 0, 0, 9]), None);
        assert_eq!(family_name(&HEADING[..64]), None);
    }
}
