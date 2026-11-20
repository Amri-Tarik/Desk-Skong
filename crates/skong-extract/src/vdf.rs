//! Valve's text KeyValues format (`libraryfolders.vdf`, `appmanifest_*.acf`): quoted
//! keys and values, nested `{ }` blocks, `//` comments.
use anyhow::{Result, bail};

#[derive(Clone, Debug, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    /// Child by key (case-insensitive, as Steam treats them).
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(kv) => kv.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            Vdf::Str(_) => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            Vdf::Obj(_) => None,
        }
    }

    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(kv) => kv,
            Vdf::Str(_) => &[],
        }
    }
}

enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokens(text: &str) -> Result<Vec<Tok>> {
    let mut out = Vec::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '"' => {
                let mut s = String::new();
                loop {
                    match it.next() {
                        Some('"') => break,
                        Some('\\') => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(c) => s.push(c),
                            None => bail!("unterminated string"),
                        },
                        Some(c) => s.push(c),
                        None => bail!("unterminated string"),
                    }
                }
                out.push(Tok::Str(s));
            }
            '/' if it.peek() == Some(&'/') => {
                for c in it.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            // Unquoted tokens (rare): up to whitespace or a brace.
            c => {
                let mut s = String::from(c);
                while let Some(&c) = it.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    s.push(c);
                    it.next();
                }
                out.push(Tok::Str(s));
            }
        }
    }
    Ok(out)
}

/// Parse a document: a sequence of `key value` / `key { … }` pairs.
pub fn parse(text: &str) -> Result<Vdf> {
    let toks = tokens(text.trim_start_matches('\u{feff}'))?;
    let mut pos = 0;
    let root = block(&toks, &mut pos, false)?;
    Ok(root)
}

fn block(toks: &[Tok], pos: &mut usize, nested: bool) -> Result<Vdf> {
    let mut kv = Vec::new();
    loop {
        match toks.get(*pos) {
            None if nested => bail!("missing '}}'"),
            None => return Ok(Vdf::Obj(kv)),
            Some(Tok::Close) if nested => {
                *pos += 1;
                return Ok(Vdf::Obj(kv));
            }
            Some(Tok::Close) | Some(Tok::Open) => bail!("unexpected brace"),
            Some(Tok::Str(k)) => {
                *pos += 1;
                let v = match toks.get(*pos) {
                    Some(Tok::Str(v)) => {
                        *pos += 1;
                        Vdf::Str(v.clone())
                    }
                    Some(Tok::Open) => {
                        *pos += 1;
                        block(toks, pos, true)?
                    }
                    _ => bail!("key '{k}' has no value"),
                };
                kv.push((k.clone(), v));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let text = "\u{feff}\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"228980\"\t\t\"1\"\n\t\t}\n\t}\n\t// a comment\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"E:\\\\SteamLibrary\"\n\t\t\"apps\" { \"367520\" \"9\" }\n\t}\n}\n";
        let v = parse(text).unwrap();
        let libs = v.get("LibraryFolders").unwrap();
        assert_eq!(libs.entries().len(), 2);
        assert_eq!(libs.get("0").unwrap().get("path").unwrap().str(), Some("C:\\Program Files (x86)\\Steam"));
        let lib1 = libs.get("1").unwrap();
        assert_eq!(lib1.get("path").unwrap().str(), Some("E:\\SteamLibrary"));
        assert!(lib1.get("apps").unwrap().get("367520").is_some());
    }

    #[test]
    fn parses_the_old_flat_format_and_rejects_broken_input() {
        let v = parse("\"LibraryFolders\" { \"TimeNextStatsReport\" \"1\" \"1\" \"D:\\\\Games\" }").unwrap();
        assert_eq!(v.get("libraryfolders").unwrap().get("1").unwrap().str(), Some("D:\\Games"));
        assert!(parse("\"a\" { \"b\" \"c\"").is_err());
        assert!(parse("\"a\" \"unterminated").is_err());
        assert!(parse("\"a\"").is_err());
        assert!(parse("}").is_err());
    }
}
