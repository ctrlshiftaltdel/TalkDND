//! Handouts: book images (maps, letters, items) shown to the players.
//!
//! The registry lives next to the locally converted book
//! (`Adventures/…/.md/<Book>/handouts.yaml`, gitignored like the book) and is
//! built from pdf2md's `index.json` (maps, figures, art) and `img/`. Every
//! entry is DM-only until the DM marks it player-safe; DM-only images are
//! never exported. Shown handouts are copied to `Saves/<slot>/handouts/`,
//! recorded in `Saves/<slot>/handouts.yaml` and in the reveal ledger.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::{chargen, reveal, save};
use combat_sim::fsutil::atomic_write;

pub const REGISTRY: &str = "handouts.yaml";
const SHOWN: &str = "handouts.yaml";

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Registry {
    #[serde(default)]
    pub book: String,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Entry {
    pub id: String,
    /// Image path relative to the book folder (`img/p0012-00.webp`).
    pub img: String,
    #[serde(default)]
    pub page: u32,
    /// map, figure (whole page) or art.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub caption: String,
    /// Nearest heading (chapter / area) for the page.
    #[serde(default)]
    pub area: String,
    /// `dm` (default, never shown) or `player`.
    #[serde(default = "dm")]
    pub visibility: String,
    /// Player-safe region `x,y,w,h` (fractions 0-1 or pixels). When set, only
    /// this region is ever exported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<String>,
    /// DM's reason or checklist note for the marking.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The last conversion no longer has this image; the entry and its DM
    /// markings are kept so a reconversion does not lose them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}

fn dm() -> String {
    "dm".into()
}

impl Entry {
    pub fn player(&self) -> bool {
        self.visibility == "player"
    }
    fn title(&self) -> String {
        if !self.caption.is_empty() {
            self.caption.clone()
        } else if !self.area.is_empty() {
            format!("{}（p.{}）", self.area, self.page)
        } else {
            format!("p.{} {}", self.page, self.kind)
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Shown {
    #[serde(default)]
    shown: Vec<ShownItem>,
}

#[derive(Serialize, Deserialize, Clone)]
struct ShownItem {
    id: String,
    caption: String,
    file: String,
    date: String,
}

// ---------- locating the adventure's converted books ----------

fn md_dirs(parent: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(parent)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("index.json").is_file())
        .collect();
    v.sort();
    v
}

/// Converted book folders of the save's adventure, or of `--book`
/// (a path, or a book / adventure name).
pub fn book_dirs(root: &Path, save: &Path, book: Option<&str>) -> Result<Vec<PathBuf>> {
    let adv = root.join("Adventures");
    let want = match book {
        Some(b) => {
            let p = PathBuf::from(b);
            for c in [p.clone(), root.join(&p)] {
                if c.join("index.json").is_file() {
                    return Ok(vec![c]);
                }
            }
            chargen::slug(b)
        }
        None => crate::start::adventure_slug(save).context(
            "the save has no adventure; set it (`talkdnd chargen set adventure …`) or pass --book",
        )?,
    };
    let mut out = md_dirs(&adv.join(&want).join(".md"));
    let mut all: Vec<PathBuf> = md_dirs(&adv.join(".md"));
    if let Ok(rd) = fs::read_dir(&adv) {
        for e in rd.flatten() {
            all.extend(md_dirs(&e.path().join(".md")));
        }
    }
    for d in all {
        let name = d
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if chargen::slug(&name) == want && !out.contains(&d) {
            out.push(d);
        }
    }
    if out.is_empty() {
        bail!("no converted book for `{want}` under Adventures/ (run pdf2md first, or pass --book <folder>)");
    }
    Ok(out)
}

// ---------- building the registry ----------

fn s(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn page_of(v: &serde_json::Value) -> u32 {
    s(&v["page"]).parse().unwrap_or(0)
}

fn id_of(img: &str) -> String {
    Path::new(img)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Captions that say the map is the players' version.
fn player_caption(c: &str) -> bool {
    let l = c.to_lowercase();
    (l.contains("player") || c.contains("玩家")) && !l.contains("dm") && !c.contains("城主")
}

/// Build or refresh a book's registry from its index; DM edits (visibility,
/// captions, crops, notes) are kept.
pub fn build(dir: &Path) -> Result<(Registry, usize)> {
    let idx: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("index.json")).context("index.json")?)?;
    let mut heads: Vec<(u32, String)> = idx["headings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|h| h["level"].as_u64().unwrap_or(9) <= 3)
        .map(|h| (page_of(h), s(&h["text"])))
        .collect();
    heads.sort_by_key(|h| h.0);
    let area = |p: u32| {
        heads
            .iter()
            .rev()
            .find(|h| h.0 <= p)
            .map(|h| h.1.clone())
            .unwrap_or_default()
    };
    let mut found: Vec<Entry> = Vec::new();
    let mut add = |img: String, page: u32, kind: &str, caption: String| {
        if img.is_empty() || found.iter().any(|e| e.img == img) {
            return;
        }
        let visibility = if kind == "map" && player_caption(&caption) {
            "player"
        } else {
            "dm"
        };
        found.push(Entry {
            id: id_of(&img),
            img,
            page,
            kind: kind.into(),
            caption,
            area: area(page),
            visibility: visibility.into(),
            ..Default::default()
        });
    };
    for m in idx["maps"].as_array().into_iter().flatten() {
        add(s(&m["img"]), page_of(m), "map", s(&m["caption"]));
    }
    for a in idx["art"].as_array().into_iter().flatten() {
        add(s(&a["img"]), page_of(a), "art", String::new());
    }
    for f in idx["figures"].as_array().into_iter().flatten() {
        add(s(&f["img"]), page_of(f), "figure", String::new());
    }
    // Images in img/ the index does not list (older conversions).
    let mut extra: Vec<String> = fs::read_dir(dir.join("img"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".webp") || n.ends_with(".png") || n.ends_with(".jpg"))
        .collect();
    extra.sort();
    for n in extra {
        let page = n
            .trim_start_matches('p')
            .split(['-', '.'])
            .next()
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        add(format!("img/{n}"), page, "figure", String::new());
    }
    let old = load(dir).unwrap_or_default();
    let present: Vec<String> = found.iter().map(|e| e.img.clone()).collect();
    let mut reg = Registry {
        book: dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        entries: Vec::new(),
    };
    let mut new = 0;
    for mut e in found {
        if let Some(o) = old.entries.iter().find(|o| o.img == e.img) {
            e.id = o.id.clone();
            e.visibility = o.visibility.clone();
            e.crop = o.crop.clone();
            e.note = o.note.clone();
            if !o.caption.is_empty() {
                e.caption = o.caption.clone();
            }
        } else {
            new += 1;
        }
        reg.entries.push(e);
    }
    // Keep old entries whose image vanished (a reconversion renamed or
    // dropped it) so the DM's visibility, crop, caption and note survive.
    for o in old.entries {
        if !present.contains(&o.img) {
            let mut e = o;
            e.missing = true;
            reg.entries.push(e);
        }
    }
    reg.entries
        .sort_by(|a, b| (a.page, &a.id).cmp(&(b.page, &b.id)));
    store(dir, &reg)?;
    Ok((reg, new))
}

pub fn load(dir: &Path) -> Result<Registry> {
    let p = dir.join(REGISTRY);
    if !p.is_file() {
        return Ok(Registry::default());
    }
    serde_norway::from_str(&fs::read_to_string(&p)?)
        .with_context(|| format!("invalid {}", p.display()))
}

fn store(dir: &Path, r: &Registry) -> Result<()> {
    let text = format!(
        "# TalkDND handout registry (local, from the converted book; never commit).\n# visibility: dm (default, never shown) | player. Change with `talkdnd handout mark`.\n{}",
        serde_norway::to_string(r)?
    );
    atomic_write(dir.join(REGISTRY), text.as_bytes())?;
    Ok(())
}

/// All registries of the adventure, built on first use.
fn registries(dirs: &[PathBuf]) -> Result<Vec<(PathBuf, Registry)>> {
    let mut out = Vec::new();
    for d in dirs {
        let r = if d.join(REGISTRY).is_file() {
            load(d)?
        } else {
            build(d)?.0
        };
        out.push((d.clone(), r));
    }
    Ok(out)
}

fn find<'a>(regs: &'a [(PathBuf, Registry)], q: &str) -> Result<(&'a PathBuf, &'a Entry)> {
    let all: Vec<(&PathBuf, &Entry)> = regs
        .iter()
        .flat_map(|(d, r)| r.entries.iter().map(move |e| (d, e)))
        .collect();
    let exact: Vec<_> = all.iter().filter(|(_, e)| e.id == q).collect();
    if exact.len() == 1 {
        return Ok(*exact[0]);
    }
    let ql = q.to_lowercase();
    let hits: Vec<_> = all
        .iter()
        .filter(|(_, e)| {
            e.caption.to_lowercase().contains(&ql)
                || e.area.to_lowercase().contains(&ql)
                || e.note.to_lowercase().contains(&ql)
        })
        .collect();
    match hits.len() {
        0 => bail!("no handout matches `{q}` (`talkdnd handout list --dm`)"),
        1 => Ok(*hits[0]),
        _ => {
            let player: Vec<_> = hits.iter().filter(|(_, e)| e.player()).collect();
            if player.len() == 1 {
                return Ok(**player[0]);
            }
            let ids: Vec<String> = hits
                .iter()
                .take(12)
                .map(|(_, e)| format!("{} [{}] {}", e.id, e.visibility, e.title()))
                .collect();
            bail!(
                "`{q}` matches {} handouts; use an id:\n  {}",
                hits.len(),
                ids.join("\n  ")
            )
        }
    }
}

// ---------- crop ----------

/// Parse `x,y,w,h`: all <= 1 means fractions of the image, else pixels.
pub fn crop_rect(spec: &str, w: u32, h: u32) -> Result<(u32, u32, u32, u32)> {
    let v: Vec<f64> = spec
        .split(',')
        .map(|x| x.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .context("--crop x,y,w,h (fractions 0-1 or pixels)")?;
    if v.len() != 4 || v.iter().any(|x| *x < 0.0) {
        bail!("--crop needs four non-negative numbers x,y,w,h");
    }
    let frac = v.iter().all(|x| *x <= 1.0);
    let (x, y, cw, ch) = if frac {
        (
            v[0] * w as f64,
            v[1] * h as f64,
            v[2] * w as f64,
            v[3] * h as f64,
        )
    } else {
        (v[0], v[1], v[2], v[3])
    };
    let (x, y) = (x.round() as u32, y.round() as u32);
    let cw = (cw.round() as u32).min(w.saturating_sub(x));
    let ch = (ch.round() as u32).min(h.saturating_sub(y));
    if cw == 0 || ch == 0 {
        bail!("--crop region is outside the {w}x{h} image");
    }
    Ok((x, y, cw, ch))
}

// ---------- commands ----------

pub struct ShowOpts<'a> {
    pub crop: Option<&'a str>,
    pub png: bool,
}

/// Path to an entry's image, refusing entries whose image is gone.
fn image_of(dir: &Path, e: &Entry) -> Result<PathBuf> {
    let src = dir.join(&e.img);
    if e.missing || !src.is_file() {
        bail!(
            "handout {} has no image in this conversion ({}); reconvert the book (`pdf2md run`) and run `talkdnd handout build`",
            e.id,
            src.display()
        );
    }
    Ok(src)
}

/// Export a player-safe handout to the save and record it. Returns output lines.
pub fn show(
    root: &Path,
    save: &Path,
    book: Option<&str>,
    q: &str,
    o: &ShowOpts,
) -> Result<Vec<String>> {
    let dirs = book_dirs(root, save, book)?;
    let regs = registries(&dirs)?;
    let (dir, e) = find(&regs, q)?;
    if !chargen::valid_id(&e.id) {
        bail!(
            "handout id `{}` is not a safe file name; fix {} (`talkdnd handout build`)",
            e.id,
            dir.join(REGISTRY).display()
        );
    }
    let src = image_of(dir, e)?;
    if !e.player() {
        bail!(
            "handout {} is DM-only and is never shown. Look at it yourself (`{}`); if it really has no secret doors, traps, monster or treasure keys or unrevealed names, mark it (`talkdnd handout mark {} --player [--crop x,y,w,h]`), otherwise describe it in words",
            e.id,
            src.display(),
            e.id
        );
    }
    let crop = match (&e.crop, o.crop) {
        (Some(_), Some(_)) => bail!(
            "handout {} is player-safe only as its registered crop ({}); drop --crop",
            e.id,
            e.crop.as_deref().unwrap_or("")
        ),
        (Some(c), None) => Some(c.as_str()),
        (None, c) => c,
    };
    let out_dir = save.join("handouts");
    fs::create_dir_all(&out_dir)?;
    let ext = src
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let suffix = if crop.is_some() { "-crop" } else { "" };
    let dest = if crop.is_some() || o.png {
        let img = image::open(&src).with_context(|| format!("decode {}", src.display()))?;
        let img = match crop {
            Some(c) => {
                let (x, y, w, h) = crop_rect(c, img.width(), img.height())?;
                img.crop_imm(x, y, w, h)
            }
            None => img,
        };
        let d = out_dir.join(format!("{}{suffix}.png", e.id));
        img.save(&d)
            .with_context(|| format!("write {}", d.display()))?;
        d
    } else {
        let d = out_dir.join(format!("{}.{ext}", e.id));
        fs::copy(&src, &d)?;
        d
    };
    let rel = dest
        .strip_prefix(root)
        .unwrap_or(&dest)
        .to_string_lossy()
        .replace('\\', "/");
    let title = e.title();
    // Record: save list, reveal ledger, session log.
    let sp = save.join(SHOWN);
    let mut shown: Shown = if sp.is_file() {
        serde_norway::from_str(&fs::read_to_string(&sp)?).unwrap_or_default()
    } else {
        Shown::default()
    };
    shown.shown.retain(|x| x.id != e.id);
    shown.shown.push(ShownItem {
        id: e.id.clone(),
        caption: title.clone(),
        file: rel.clone(),
        date: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
    });
    let text = format!(
        "# TalkDND handouts shown to the players (`talkdnd handout show`).\n{}",
        serde_norway::to_string(&shown)?
    );
    atomic_write(&sp, text.as_bytes())?;
    let mut lines = Vec::new();
    match reveal::add(
        save,
        &format!("资料：{title}"),
        &format!("handout {}", e.id),
        &[format!("handout:{}", e.id)],
    ) {
        Ok(l) => lines.push(l),
        Err(_) => {
            if let Some(log) = save::latest_log(save) {
                save::append(
                    &log,
                    &format!(
                        "- [{}] 再次展示资料 {}：{title}",
                        chrono::Local::now().format("%H:%M"),
                        e.id
                    ),
                )?;
            }
        }
    }
    lines.push(format!("handout {}: {title}", e.id));
    lines.push(format!("file: {}", dest.display()));
    lines.push(format!("markdown: ![{title}](<{rel}>)"));
    Ok(lines)
}

/// Handouts already shown to the players (id, caption, date, file).
pub fn shown_lines(save: &Path) -> Vec<String> {
    let sp = save.join(SHOWN);
    let shown: Shown = if sp.is_file() {
        serde_norway::from_str(&fs::read_to_string(&sp).unwrap_or_default()).unwrap_or_default()
    } else {
        Shown::default()
    };
    shown
        .shown
        .iter()
        .map(|x| format!("{} {}（{}）→ {}", x.id, x.caption, x.date, x.file))
        .collect()
}

pub fn list(root: &Path, save: &Path, book: Option<&str>, dm: bool) -> Result<Vec<String>> {
    if !dm {
        let sp = save.join(SHOWN);
        let shown: Shown = if sp.is_file() {
            serde_norway::from_str(&fs::read_to_string(&sp)?).unwrap_or_default()
        } else {
            Shown::default()
        };
        if shown.shown.is_empty() {
            return Ok(vec!["no handouts shown yet".into()]);
        }
        return Ok(shown
            .shown
            .iter()
            .map(|x| format!("{} {}（{}）→ {}", x.id, x.caption, x.date, x.file))
            .collect());
    }
    let dirs = book_dirs(root, save, book)?;
    let mut out = Vec::new();
    for (d, r) in registries(&dirs)? {
        out.push(format!("# {}（{}）", r.book, d.join(REGISTRY).display()));
        for e in &r.entries {
            out.push(format!(
                "{} [{}] p.{} {} {}{}{}{}",
                e.id,
                e.visibility,
                e.page,
                e.kind,
                e.title(),
                if e.missing { " (missing)" } else { "" },
                e.crop
                    .as_deref()
                    .map(|c| format!(" crop={c}"))
                    .unwrap_or_default(),
                if e.note.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", e.note)
                }
            ));
        }
    }
    Ok(out)
}

pub fn rebuild(root: &Path, save: &Path, book: Option<&str>) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for d in book_dirs(root, save, book)? {
        let (r, new) = build(&d)?;
        let missing = r.entries.iter().filter(|e| e.missing).count();
        let p = r
            .entries
            .iter()
            .filter(|e| e.player() && !e.missing)
            .count();
        out.push(format!(
            "{}: {} images ({} new), {} player-safe, {} DM-only, {} missing → {}",
            r.book,
            r.entries.len(),
            new,
            p,
            r.entries.len() - p - missing,
            missing,
            d.join(REGISTRY).display()
        ));
        if missing > 0 {
            out.push(format!(
                "warning: {missing} handout image(s) are gone from this conversion; the entries and their markings are kept (shown as `(missing)`)"
            ));
        }
    }
    Ok(out)
}

pub struct Mark<'a> {
    pub player: bool,
    pub crop: Option<&'a str>,
    pub caption: Option<&'a str>,
    pub note: Option<&'a str>,
}

pub fn mark(root: &Path, save: &Path, book: Option<&str>, id: &str, m: &Mark) -> Result<String> {
    let dirs = book_dirs(root, save, book)?;
    let regs = registries(&dirs)?;
    let (dir, _) = find(&regs, id)?;
    let dir = dir.clone();
    let mut r = load(&dir)?;
    let e =
        r.entries.iter_mut().find(|e| e.id == id).with_context(|| {
            format!("use the exact id (`talkdnd handout list --dm`), not `{id}`")
        })?;
    if let Some(c) = m.crop {
        if c.is_empty() {
            e.crop = None;
        } else {
            crop_rect(c, 10_000, 10_000)?;
            e.crop = Some(c.to_string());
        }
    }
    e.visibility = if m.player { "player" } else { "dm" }.into();
    if let Some(c) = m.caption {
        e.caption = c.to_string();
    }
    if let Some(n) = m.note {
        e.note = n.to_string();
    }
    let line = format!(
        "handout {} → {}{}：{}",
        e.id,
        e.visibility,
        e.crop
            .as_deref()
            .map(|c| format!(" crop={c}"))
            .unwrap_or_default(),
        e.title()
    );
    store(&dir, &r)?;
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_fraction_and_pixels() {
        assert_eq!(crop_rect("0,0,0.5,0.5", 200, 100).unwrap(), (0, 0, 100, 50));
        assert_eq!(
            crop_rect("10,10,500,20", 200, 100).unwrap(),
            (10, 10, 190, 20)
        );
        assert!(crop_rect("300,0,10,10", 200, 100).is_err());
        assert!(crop_rect("1,2,3", 200, 100).is_err());
    }

    #[test]
    fn player_caption_rule() {
        assert!(player_caption("MAP 2.1: Village (Player Version)"));
        assert!(!player_caption("MAP 2.1: Village"));
        assert!(!player_caption("Player and DM map"));
    }

    #[test]
    fn build_keeps_entries_whose_image_vanished() {
        let d = std::env::temp_dir().join(format!("handout-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("img")).unwrap();
        fs::write(d.join("img/p0010-00.webp"), b"img").unwrap();
        fs::write(
            d.join("index.json"),
            serde_json::json!({
                "headings": [{"page": 1, "level": 1, "text": "Chapter 1"}],
                "maps": [{"img": "img/p0010-00.webp", "page": 10, "caption": "Map 1: The Vault"}],
                "art": [],
                "figures": []
            })
            .to_string(),
        )
        .unwrap();
        let (mut reg, _) = build(&d).unwrap();
        assert_eq!(reg.entries.len(), 1);
        reg.entries[0].visibility = "player".into();
        reg.entries[0].crop = Some("0,0,1,1".into());
        reg.entries[0].note = "players need this".into();
        store(&d, &reg).unwrap();

        // A reconversion renames/drops the image and no longer lists it.
        fs::remove_file(d.join("img/p0010-00.webp")).unwrap();
        fs::write(
            d.join("index.json"),
            serde_json::json!({"headings": [], "maps": [], "art": [], "figures": []}).to_string(),
        )
        .unwrap();
        build(&d).unwrap();

        let reg = load(&d).unwrap();
        let e = reg
            .entries
            .iter()
            .find(|e| e.id == "p0010-00")
            .expect("entry kept");
        assert!(e.missing, "{e:?}");
        assert_eq!(e.visibility, "player");
        assert_eq!(e.crop.as_deref(), Some("0,0,1,1"));
        assert_eq!(e.note, "players need this");
        assert_eq!(e.img, "img/p0010-00.webp");
        let err = image_of(&d, e).unwrap_err().to_string();
        assert!(err.contains("no image"), "{err}");
        let _ = fs::remove_dir_all(&d);
    }
}
