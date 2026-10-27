//! Integration tests for the read-only knowledge API (TASK-0113).
//!
//! Every fixture is synthetic: no real save, DM secret, book text or personal
//! path is used. The tests call the public library API and, for one smoke
//! test, the real `talkdnd` binary.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use talkdnd::knowledge::{cite, context, get, related, search, Entry, Scope};

const SLOT: &str = "2030-01-01-01";

fn temp_root(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "talkdnd-knowledge-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(p.join("Saves")).unwrap();
    fs::write(p.join("AGENTS.md"), "synthetic play root\n").unwrap();
    p
}

fn save(root: &Path) -> PathBuf {
    root.join("Saves").join(SLOT)
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}

fn structured(book: &str, pdf: &str, headings: &[(&str, u32)]) -> String {
    let hs: Vec<String> = headings
        .iter()
        .map(|(title, page)| {
            format!(
                "{{\"level\":2,\"title\":{},\"page\":{page},\"anchor\":\"p{page}\"}}",
                serde_json::to_string(title).unwrap()
            )
        })
        .collect();
    format!(
        "{{\n  \"version\": 1,\n  \"book\": {},\n  \"source\": {{\"path\": {}}},\n  \"headings\": [{}],\n  \"chapters\": [],\n  \"maps\": [],\n  \"figures\": [],\n  \"statblocks\": [],\n  \"low_quality_pages\": []\n}}\n",
        serde_json::to_string(book).unwrap(),
        serde_json::to_string(pdf).unwrap(),
        hs.join(",")
    )
}

/// The main synthetic campaign: one entry of every authority, both visible and
/// hidden, plus a small book/adventure/SRD index.
fn fixture(tag: &str) -> PathBuf {
    let root = temp_root(tag);
    let save = save(&root);
    for dir in ["Sessions", "Characters", "combat", "sheets"] {
        fs::create_dir_all(save.join(dir)).unwrap();
    }
    write(
        &save.join("Sessions/2030-01-01-session-01.md"),
        "# synthetic session\n",
    );
    write(
        &save.join("combat/campaign.yaml"),
        "party: [hero]\nruleset: \"5e-srd\"\nadventures: [../../../Adventures/test-adventure/combat]\n",
    );
    write(
        &save.join("Characters/Hero.md"),
        "# Hero\n\n```combat yaml\nid: hero\nname: Hero\nkind: pc\nlevel: 1\nhp: 10\nac: 16\nnotes: \"fighter 1\"\n```\n",
    );
    write(
        &save.join("progress.yaml"),
        "version: 1\nxp: {hero: 0}\npurses: {hero: {cp: 0}}\nscene: 宴会厅\ntrack:\n  - {name: First Job, level: 1}\n  - {name: Second Job, level: 3}\n",
    );
    write(
        &save.join("entities.yaml"),
        "version: 1\nentities:\n  - {id: E-001, type: npc, name: \"Synthetic Mira\", aliases: [Mira], first_seen: 2030-01-01-session-01.md, status: active}\n  - {id: E-002, type: location, name: \"Synthetic Vault\", aliases: [], first_seen: 2030-01-01-session-01.md, status: active}\n  - {id: E-003, type: npc, name: \"TOP-SECRET-ENTITY\", aliases: [], first_seen: 2030-01-01-session-01.md, status: active, dm_only: true}\n  - {id: E-004, type: npc, name: \"TOP-SECRET-PLAYER\", aliases: [], first_seen: 2030-01-01-session-01.md, status: active}\n  - {id: E-005, type: npc, name: \"TOP-SECRET-DMSECRET\", aliases: [], first_seen: 2030-01-01-session-01.md, status: active}\n  - {id: E-006, type: npc, name: \"TOP-SECRET-MANUAL\", aliases: [], first_seen: 2030-01-01-session-01.md, status: active}\n",
    );
    write(
        &save.join("relations.yaml"),
        "version: 1\nrelations:\n  - {id: R-001, type: ally, from: E-001, to: E-002, since_session: 2030-01-01-session-01.md, source: synthetic, note: synthetic relation}\n  - {id: R-002, type: enemy, from: E-001, to: E-003, since_session: 2030-01-01-session-01.md, dm_only: true}\n",
    );
    write(
        &save.join("memory.yaml"),
        "version: 1\nentries:\n  - {id: M-001, text: \"Synthetic memory about Mira\", source: player-confirmed, added_at: \"2030-01-01 10:00\", tags: [\"npc:E-001\"]}\n  - {id: M-002, text: \"TOP-SECRET-MEMORY\", source: manual, added_at: \"2030-01-01 10:01\", tags: []}\n",
    );
    write(
        &save.join("reveals.yaml"),
        "version: 1\nfacts:\n  - {n: 1, date: \"2030-01-01 10:00\", fact: \"Synthetic fact about Mira\", source: synthetic, tags: [\"npc:E-001\"]}\n  - {n: 2, date: \"2030-01-01 10:01\", fact: \"Synthetic fact about the vault\", source: synthetic, tags: []}\n",
    );
    write(
        &save.join("original.yaml"),
        "version: 2\nentries:\n  - {id: O-001, type: npc, name: \"Synthetic Original\", rarity: common, chapter: First Job, note: \"Mira's lantern\", logged: synthetic}\n  - {id: O-002, type: npc, name: \"TOP-SECRET-HIDDEN-ORIGINAL\", rarity: common, chapter: First Job, hidden: \"TOP-SECRET-HIDDEN\", logged: synthetic}\n  - {id: O-003, type: npc, name: \"Synthetic Revealed\", rarity: common, chapter: First Job, hidden: \"Synthetic revealed secret\", logged: synthetic}\n",
    );
    write(
        &save.join("identified.yaml"),
        "version: 1\nentries:\n  - {name: Goblin, identified: true, how: check, reason: synthetic, date: \"2030-01-01 10:00\"}\n  - {name: Ogre, identified: false, appearance: \"a large shape\", date: \"2030-01-01 10:00\"}\n",
    );
    write(
        &save.join("secret-terms.yaml"),
        "version: 1\nterms:\n  - {term: TOP-SECRET-MANUAL, chapter: 1, note: synthetic, added: \"2030-01-01 10:00\"}\n  - {term: TOP-SECRET-MEMORY, chapter: 1, note: synthetic, added: \"2030-01-01 10:00\"}\n",
    );
    write(
        &save.join("handouts.yaml"),
        "shown:\n  - {id: h-player, caption: \"Synthetic Player Map\", file: \"Saves/2030-01-01-01/handouts/h-player.webp\", date: \"2030-01-01 10:00\"}\n",
    );
    write(&save.join("dm-secrets.md"), "- term: TOP-SECRET-DMSECRET\n");
    write(&save.join("dm-plan.md"), "- term: TOP-SECRET-PLAN\n");

    // Rulings: table copy plus defaults for the version file.
    write(
        &root.join("Rules/rulings.md"),
        "# Synthetic rulings\n\n## A. Synthetic\n\n### R-01 Synthetic Ruling\n\nSynthetic ruling body about lanterns.\n\n### R-02 Another Ruling\n\nAnother synthetic body.\n",
    );
    write(
        &root.join("Rules/defaults/rulings-2014.md"),
        "# Synthetic 2014 rulings\n\n### V-01 Synthetic Version Ruling\n\nSynthetic version body.\n",
    );

    // SRD pack: one rule section and one ability.
    write(
        &root.join("Rules/combat/2014/srd/synthetic.pack.json"),
        r#"{
  "pack": {"id": "synthetic-srd", "name": "Synthetic SRD", "edition": "2014", "distribution": "srd"},
  "index": {"entries": [
    {"id": "rule-synthetic", "kind": "rule-section", "name": "Synthetic SRD Rule", "aliases": ["Lantern Rule"], "page": 12, "section": "Rules", "refs": []},
    {"id": "ability-synthetic", "kind": "ability", "name": "Synthetic SRD Ability", "aliases": [], "page": null, "section": "Spells", "refs": []}
  ]},
  "content": {
    "rule_sections": [
      {"id": "rule-synthetic", "name": "Synthetic SRD Rule", "source": "SRD 5.1", "page": 12, "section": "Rules", "tags": ["rule"], "aliases": ["Lantern Rule"], "text": "Synthetic SRD body about lanterns.", "refs": []}
    ],
    "abilities": [
      {"id": "ability-synthetic", "name": "Synthetic SRD Ability", "tags": [], "description": "Synthetic ability body.", "source": "SRD 5.1", "page": null}
    ]
  }
}
"#,
    );

    // Player book index under the save's edition.
    write(
        &root.join("Rules/2014/player-book/index/index.json"),
        "{\"pdf\": \"Player Book.pdf\"}\n",
    );
    write(
        &root.join("Rules/2014/player-book/index/Player Book.pages.md"),
        "# Player Book\n\n<a id=\"p7\"></a>\n<!-- PDF p.7 -->\n## Synthetic Book Section\n\nSynthetic book body about a lantern.\n",
    );
    write(
        &root.join("Rules/2014/player-book/index/structured.json"),
        &structured(
            "player-book",
            "Player Book.pdf",
            &[("Synthetic Book Section", 7)],
        ),
    );

    // Adventure index: one current heading, one future heading, one DM-only
    // heading and two handouts.
    write(
        &root.join("Adventures/test-adventure/index/index.json"),
        "{\"pdf\": \"Test Book.pdf\"}\n",
    );
    write(
        &root.join("Adventures/test-adventure/index/Test Book.pages.md"),
        "# Test Book\n\n<a id=\"p1\"></a>\n<!-- PDF p.1 -->\n## Chapter 1: The Inn\n\nSynthetic adventure body about Mira.\n\n<a id=\"p2\"></a>\n<!-- PDF p.2 -->\n## Chapter 9: TOP-SECRET-FUTURE\n\nFuture synthetic body.\n\n<a id=\"p3\"></a>\n<!-- PDF p.3 -->\n## DM Notes\n\nTOP-SECRET-DM body.\n",
    );
    write(
        &root.join("Adventures/test-adventure/index/structured.json"),
        r#"{
  "version": 1,
  "book": "test-adventure",
  "source": {"path": "Test Book.pdf"},
  "headings": [
    {"level":2,"title":"Chapter 1: The Inn","page":1,"anchor":"p1"},
    {"level":2,"title":"Chapter 9: TOP-SECRET-FUTURE","page":2,"anchor":"p2"},
    {"level":2,"title":"DM Notes","page":3,"anchor":"p3"},
    {"level":2,"title":"Synthetic Adventure Room","page":4,"anchor":"p4"}
  ],
  "chapters": [],
  "maps": [],
  "figures": [],
  "statblocks": [],
  "low_quality_pages": []
}
"#,
    );
    write(
        &root.join("Adventures/test-adventure/index/handouts.yaml"),
        "book: test-adventure\nentries:\n  - {id: h-player, img: img/player.webp, page: 1, kind: map, caption: \"Synthetic Player Map\", area: \"The Inn\", visibility: player}\n  - {id: h-dm, img: img/dm.webp, page: 2, kind: map, caption: \"TOP-SECRET-HANDOUT\", area: \"The Vault\", visibility: dm}\n",
    );
    root
}

fn ids(entries: &[Entry]) -> Vec<String> {
    entries.iter().map(|e| e.id.clone()).collect()
}

fn joined(entries: &[Entry]) -> String {
    entries
        .iter()
        .map(|e| format!("{} {} {} {}", e.id, e.authority, e.title, e.excerpt))
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn search_covers_six_authorities_and_scopes() {
    let root = fixture("authorities");
    let save = save(&root);
    let all = search(&root, &save, "Synthetic", Scope::All, 100, false).unwrap();
    let authorities: std::collections::BTreeSet<&str> =
        all.iter().map(|e| e.authority.as_str()).collect();
    for wanted in ["ruling", "save", "book", "srd", "adventure", "original"] {
        assert!(
            authorities.contains(wanted),
            "missing authority {wanted}: {:?}",
            ids(&all)
        );
    }
    for (scope, wanted) in [
        (Scope::Save, "save"),
        (Scope::Book, "book"),
        (Scope::Srd, "srd"),
        (Scope::Adventure, "adventure"),
        (Scope::Original, "original"),
    ] {
        let got = search(&root, &save, "Synthetic", scope, 100, false).unwrap();
        assert!(!got.is_empty(), "scope {scope:?} returned nothing");
        assert!(
            got.iter().all(|e| e.authority == wanted),
            "scope {scope:?}: {:?}",
            ids(&got)
        );
    }
    // Every result carries the fixed fields and is deterministic.
    for e in &all {
        assert!(!e.id.is_empty());
        assert!(!e.authority.is_empty());
        assert!(!e.title.is_empty());
        assert!(!e.citation.is_empty());
        assert!(!e.excerpt.is_empty());
        assert!(e.known);
        assert_eq!(e.visibility, "player");
    }
    assert_eq!(
        all,
        search(&root, &save, "Synthetic", Scope::All, 100, false).unwrap()
    );
}

#[test]
fn search_limits_are_deterministic_and_capped() {
    let root = temp_root("bulk");
    let save = save(&root);
    fs::create_dir_all(save.join("Sessions")).unwrap();
    fs::create_dir_all(save.join("combat")).unwrap();
    write(&save.join("Sessions/2030-01-01-session-01.md"), "# bulk\n");
    write(
        &save.join("combat/campaign.yaml"),
        "party: [hero]\nruleset: \"5e-srd\"\n",
    );
    let mut entities = String::from("version: 1\nentities:\n");
    for i in 1..=1000 {
        entities.push_str(&format!(
            "  - {{id: E-{i:03}, type: npc, name: \"Bulk {i:04}\", aliases: [], first_seen: 2030-01-01-session-01.md, status: active}}\n"
        ));
    }
    write(&save.join("entities.yaml"), &entities);
    let mut reveals = String::from("version: 1\nfacts:\n");
    for i in 1..=1000 {
        reveals.push_str(&format!(
            "  - {{n: {i}, date: \"2030-01-01 10:00\", fact: \"Bulk fact {i:04}\", source: synthetic, tags: []}}\n"
        ));
    }
    write(&save.join("reveals.yaml"), &reveals);

    let default = search(&root, &save, "Bulk", Scope::All, 20, false).unwrap();
    assert_eq!(default.len(), 20);
    let five = search(&root, &save, "Bulk", Scope::All, 5, false).unwrap();
    assert_eq!(five.len(), 5);
    let capped = search(&root, &save, "Bulk", Scope::All, 0, false).unwrap();
    assert!(capped.len() <= 500, "hard cap: {}", capped.len());
    assert!(search(&root, &save, "Bulk", Scope::All, 1001, false).is_err());

    let lines = context(&root, &save, "E-001").unwrap();
    assert!(
        lines.len() < 100,
        "context must stay bounded: {}",
        lines.len()
    );
    assert!(lines.iter().any(|l| l.contains("已知")), "{lines:?}");
}

#[test]
fn get_returns_one_entry_and_fails_closed() {
    let root = fixture("get");
    let save = save(&root);
    let e = get(&root, &save, "save:entity:E-001", false).unwrap();
    assert_eq!(e.id, "save:entity:E-001");
    assert_eq!(e.authority, "save");
    assert!(e.title.contains("Synthetic Mira"));
    assert!(e.citation.contains("entities.yaml"));
    assert_eq!(e.visibility, "player");
    assert!(e.known);

    for bad in [
        "not-an-id",
        "save:entity:E-999",
        "book:2014:player-book:p999",
        "ruling:2014:R-99",
        "",
    ] {
        assert!(get(&root, &save, bad, false).is_err(), "accepted {bad}");
    }
    // A hidden entry is refused without the flag and returned with it.
    assert!(get(&root, &save, "save:entity:E-003", false).is_err());
    let dm = get(&root, &save, "save:entity:E-003", true).unwrap();
    assert_eq!(dm.visibility, "dm");
    assert!(!dm.known);

    // Corrupt ledgers fail closed, not panic.
    write(&save.join("entities.yaml"), "version: 1\nentities: [oops\n");
    assert!(search(&root, &save, "Synthetic", Scope::All, 20, false).is_err());
    assert!(get(&root, &save, "save:entity:E-001", false).is_err());
}

#[test]
fn corrupt_external_indexes_fail_closed() {
    let root = fixture("corrupt-index");
    let broken_save = save(&root);
    write(
        &root.join("Rules/2014/player-book/index/structured.json"),
        "{not json",
    );
    assert!(search(&root, &broken_save, "Synthetic", Scope::All, 20, false).is_err());
    // Other roots still load normally.
    let clean = fixture("corrupt-index-clean");
    assert!(
        !search(&clean, &save(&clean), "Synthetic", Scope::All, 20, false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cite_uses_relative_citations_and_never_the_body() {
    let root = fixture("cite");
    let save = save(&root);
    let cases = [
        ("ruling:common:R-01", "Rules/rulings.md", "R-01"),
        ("save:entity:E-001", "entities.yaml", "E-001"),
        ("book:2014:player-book:p7", "Player Book", "p.7"),
        ("srd:rule-section:rule-synthetic", "SRD", "rule-synthetic"),
        (
            "adventure:test-adventure:p1",
            "Adventures/test-adventure",
            "#p1",
        ),
        ("original:O-001", "original.yaml", "O-001"),
    ];
    for (id, a, b) in cases {
        let c = cite(&root, &save, id).unwrap();
        assert!(c.contains(a) && c.contains(b), "{id}: {c}");
        assert!(!c.contains("Synthetic SRD body"), "{id}: {c}");
        assert!(!c.contains(&root.display().to_string()), "{id}: {c}");
    }
    // cite has no --include-hidden: a hidden id is never cited.
    assert!(cite(&root, &save, "save:entity:E-003").is_err());
    assert!(cite(&root, &save, "original:O-002").is_err());
    assert!(cite(&root, &save, "save:memory:M-002").is_err());
}

#[test]
fn hidden_filtering_never_leaks_secret_terms() {
    let root = fixture("hidden");
    let save = save(&root);
    let default = search(&root, &save, "TOP-SECRET", Scope::All, 100, false).unwrap();
    let text = joined(&default);
    assert!(text.contains("TOP-SECRET-PLAYER"), "{text}");
    for forbidden in [
        "TOP-SECRET-ENTITY",
        "TOP-SECRET-HIDDEN-ORIGINAL",
        "TOP-SECRET-FUTURE",
        "TOP-SECRET-HANDOUT",
        "TOP-SECRET-DM",
        "TOP-SECRET-DMSECRET",
        "TOP-SECRET-MANUAL",
        "TOP-SECRET-MEMORY",
        "TOP-SECRET-PLAN",
    ] {
        assert!(!text.contains(forbidden), "leaked {forbidden}: {text}");
    }
    assert!(default.iter().all(|e| e.visibility == "player"));

    let all = search(&root, &save, "TOP-SECRET", Scope::All, 100, true).unwrap();
    let text = joined(&all);
    for wanted in [
        "TOP-SECRET-ENTITY",
        "TOP-SECRET-HIDDEN-ORIGINAL",
        "TOP-SECRET-FUTURE",
        "TOP-SECRET-HANDOUT",
    ] {
        assert!(
            text.contains(wanted),
            "include-hidden missed {wanted}: {text}"
        );
    }
    // `dm-secrets.md`, `dm-plan.md` and manual secret terms stay out even in
    // the DM view.
    for forbidden in [
        "TOP-SECRET-DMSECRET",
        "TOP-SECRET-PLAN",
        "TOP-SECRET-MANUAL",
        "TOP-SECRET-MEMORY",
    ] {
        assert!(
            !text.contains(forbidden),
            "DM view leaked {forbidden}: {text}"
        );
    }
    for e in all.iter().filter(|e| {
        [
            "TOP-SECRET-ENTITY",
            "TOP-SECRET-HIDDEN-ORIGINAL",
            "TOP-SECRET-FUTURE",
            "TOP-SECRET-HANDOUT",
        ]
        .iter()
        .any(|w| e.title.contains(w))
    }) {
        assert_eq!(e.visibility, "dm", "{}", e.id);
        assert!(!e.known, "{}", e.id);
    }
    // The blocked entries cannot be fetched even with the flag.
    assert!(get(&root, &save, "save:entity:E-005", true).is_err());
    assert!(get(&root, &save, "save:entity:E-004", false).is_ok());
    // The hidden original can be fetched only as DM content.
    assert!(get(&root, &save, "original:O-002", false).is_err());
    let dm = get(&root, &save, "original:O-002", true).unwrap();
    assert_eq!(dm.visibility, "dm");
    assert!(dm.excerpt.contains("TOP-SECRET-HIDDEN"), "{}", dm.excerpt);
    // `dm-secrets.md` is not a source and has no id.
    assert!(get(&root, &save, "save:secret:0", true).is_err());
    // The context view applies the same boundary.
    for key in ["E-005", "E-006"] {
        let text = context(&root, &save, key).unwrap().join("\n");
        for forbidden in [
            "TOP-SECRET-DMSECRET",
            "TOP-SECRET-MANUAL",
            "TOP-SECRET-MEMORY",
        ] {
            assert!(
                !text.contains(forbidden),
                "context leaked {forbidden}: {text}"
            );
        }
    }
    assert!(context(&root, &save, "TOP-SECRET-DMSECRET").is_err());
    assert!(context(&root, &save, "E-003").is_err());
}

#[test]
fn relation_endpoint_visibility_uses_both_entities() {
    let root = fixture("relation-endpoint");
    let save = save(&root);
    // R-002 itself is player-visible, but its target E-003 is dm_only.
    write(
        &save.join("relations.yaml"),
        "version: 1\nrelations:\n  - {id: R-001, type: ally, from: E-001, to: E-002, since_session: 2030-01-01-session-01.md, source: synthetic, note: synthetic relation}\n  - {id: R-002, type: enemy, from: E-001, to: E-003, since_session: 2030-01-01-session-01.md, source: synthetic, note: \"points at TOP-SECRET-ENTITY\"}\n",
    );
    let default = search(&root, &save, "TOP-SECRET-ENTITY", Scope::All, 100, false).unwrap();
    assert!(
        !joined(&default).contains("TOP-SECRET-ENTITY"),
        "default search leaked the dm_only endpoint: {:?}",
        ids(&default)
    );
    let context_text = context(&root, &save, "E-001").unwrap().join("\n");
    assert!(
        !context_text.contains("TOP-SECRET-ENTITY"),
        "context leaked the dm_only endpoint: {context_text}"
    );
    let related_text = joined(&related(&root, &save, "save:entity:E-001", 1).unwrap());
    assert!(
        !related_text.contains("TOP-SECRET-ENTITY")
            && !related_text.contains("save:relation:R-002"),
        "related leaked the dm_only endpoint: {related_text}"
    );

    let all = search(&root, &save, "TOP-SECRET-ENTITY", Scope::All, 100, true).unwrap();
    let relation = all
        .iter()
        .find(|e| e.id == "save:relation:R-002")
        .unwrap_or_else(|| panic!("missing hidden relation: {:?}", ids(&all)));
    assert_eq!(relation.visibility, "dm");
    assert!(!relation.known);
}

#[test]
fn future_chapter_without_progress_is_hidden_by_default() {
    let root = fixture("future-no-progress");
    let save = save(&root);
    fs::remove_file(save.join("progress.yaml")).unwrap();
    // Remove every secret/DM marker from the future heading so only the
    // progress gate can hide it.
    for path in [
        root.join("Adventures/test-adventure/index/Test Book.pages.md"),
        root.join("Adventures/test-adventure/index/structured.json"),
    ] {
        let text = fs::read_to_string(&path).unwrap();
        write(&path, &text.replace("TOP-SECRET-FUTURE", "The Future Gate"));
    }
    let default = search(&root, &save, "Future Gate", Scope::Adventure, 20, false).unwrap();
    assert!(
        default.is_empty(),
        "future chapter visible without progress: {:?}",
        ids(&default)
    );
    // The first chapter / front matter stays visible.
    let current = search(&root, &save, "The Inn", Scope::Adventure, 20, false).unwrap();
    assert!(
        current
            .iter()
            .any(|e| e.id == "adventure:test-adventure:p1"),
        "current chapter was hidden: {:?}",
        ids(&current)
    );
    let all = search(&root, &save, "Future Gate", Scope::Adventure, 20, true).unwrap();
    let entry = all
        .iter()
        .find(|e| e.id == "adventure:test-adventure:p2")
        .unwrap_or_else(|| panic!("missing future chapter: {:?}", ids(&all)));
    assert_eq!(entry.visibility, "dm");
    assert!(!entry.known);
    assert!(joined(&all).contains("The Future Gate"));
}

#[test]
fn hidden_original_text_in_a_visible_relation_is_never_output() {
    let root = fixture("hidden-original-note");
    let save = save(&root);
    write(
        &save.join("relations.yaml"),
        "version: 1\nrelations:\n  - {id: R-001, type: ally, from: E-001, to: E-002, since_session: 2030-01-01-session-01.md, source: synthetic, note: \"TOP-SECRET-HIDDEN\"}\n",
    );
    let secret = "TOP-SECRET-HIDDEN";
    let default = search(&root, &save, secret, Scope::All, 100, false).unwrap();
    assert!(
        default.is_empty(),
        "default search leaked: {:?}",
        ids(&default)
    );
    let dm = search(&root, &save, secret, Scope::All, 100, true).unwrap();
    assert!(
        !dm.iter().any(|e| e.id == "save:relation:R-001"),
        "include-hidden returned the visible relation note: {:?}",
        ids(&dm)
    );
    assert!(
        dm.iter()
            .all(|e| e.visibility == "dm" || !e.excerpt.contains(secret)),
        "a player-visible entry leaked the hidden text: {:?}",
        ids(&dm)
    );
    // The hidden original itself is legitimate DM content; the leak is the
    // player-visible relation note that quotes it.
    assert!(dm.iter().any(|e| e.id == "original:O-002"));
    assert!(get(&root, &save, "save:relation:R-001", true).is_err());
    assert!(cite(&root, &save, "save:relation:R-001").is_err());
    let context_text = context(&root, &save, "E-001").unwrap().join("\n");
    assert!(
        !context_text.contains(secret),
        "context leaked: {context_text}"
    );
    let related_text = joined(&related(&root, &save, "save:entity:E-001", 1).unwrap());
    assert!(
        !related_text.contains(secret),
        "related leaked: {related_text}"
    );
}

#[test]
fn secret_bearing_ids_are_not_echoed() {
    let root = fixture("secret-id-echo");
    let save = save(&root);
    let secret = "TOP-SECRET-DMSECRET";
    for err in [
        get(&root, &save, secret, false).unwrap_err().to_string(),
        cite(&root, &save, secret).unwrap_err().to_string(),
        related(&root, &save, secret, 1).unwrap_err().to_string(),
    ] {
        assert!(!err.contains(secret), "error echoed the secret id: {err}");
    }
}

#[test]
fn context_is_ordered_and_marks_sources() {
    let root = fixture("context");
    let save = save(&root);
    let lines = context(&root, &save, "E-001").unwrap();
    let text = lines.join("\n");
    let pos = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("missing {needle}: {text}"))
    };
    let scene = pos("当前场景");
    let party = pos("队伍");
    let known = pos("已知");
    let relations = pos("关系");
    let rules = pos("规则");
    assert!(
        scene < party && party < known && known < relations && relations < rules,
        "{text}"
    );
    assert!(text.contains("宴会厅"), "{text}");
    assert!(text.contains("Hero"), "{text}");
    assert!(text.contains("Synthetic Player Map"), "{text}");
    assert!(text.contains("来源："), "{text}");
    assert!(text.contains("R-001"), "{text}");
    assert!(text.contains("R-01"), "{text}");
    // A missing key is still a structured context; nothing panics.
    let empty = context(&root, &save, "不存在的地方").unwrap();
    assert!(empty.iter().any(|l| l.contains("当前场景")), "{empty:?}");
}

#[test]
fn context_on_an_empty_save_prints_none() {
    let root = temp_root("empty-context");
    let save = save(&root);
    fs::create_dir_all(save.join("Sessions")).unwrap();
    write(&save.join("Sessions/2030-01-01-session-01.md"), "# empty\n");
    let lines = context(&root, &save, "nothing").unwrap();
    let text = lines.join("\n");
    for label in ["当前场景", "队伍", "已知", "关系", "规则"] {
        assert!(text.contains(label), "{text}");
    }
    assert!(text.matches("无").count() >= 5, "{text}");
    let empty = search(&root, &save, "anything", Scope::All, 20, false).unwrap();
    assert!(empty.is_empty());
    assert!(cite(&root, &save, "save:entity:E-001").is_err());
}

#[test]
fn related_follows_relations_and_shared_references() {
    let root = fixture("related");
    let save = save(&root);
    let one = related(&root, &save, "save:entity:E-001", 1).unwrap();
    let text = joined(&one);
    for wanted in [
        "save:relation:R-001",
        "save:entity:E-002",
        "save:reveal:1",
        "original:O-001",
    ] {
        assert!(text.contains(wanted), "depth 1 missing {wanted}: {text}");
    }
    assert_eq!(one, related(&root, &save, "save:entity:E-001", 1).unwrap());
    let two = related(&root, &save, "save:entity:E-001", 2).unwrap();
    assert!(two.len() >= one.len(), "depth 2 must not shrink");
    assert!(two.iter().all(|e| e.id != "save:entity:E-001"));
    assert!(related(&root, &save, "save:entity:E-001", 3).is_err());
    assert!(related(&root, &save, "save:entity:E-001", 0).is_err());
    assert!(related(&root, &save, "save:entity:E-003", 1).is_err());
    assert!(related(&root, &save, "save:entity:E-999", 1).is_err());
    // A memory/reveal seed follows its explicit reference tags.
    for seed in ["save:memory:M-001", "save:reveal:1"] {
        let text = joined(&related(&root, &save, seed, 1).unwrap());
        assert!(text.contains("save:entity:E-001"), "{seed}: {text}");
    }
    assert!(search(&root, &save, "   ", Scope::All, 20, false).is_err());
    assert!(cite(&root, &save, "adventure:test-adventure:handout-h-dm").is_err());
    assert!(search(
        &root,
        &save,
        "no-such-synthetic-term",
        Scope::All,
        20,
        false
    )
    .unwrap()
    .is_empty());
}

#[test]
fn cli_smoke_is_registered_and_read_only() {
    let root = fixture("cli");
    let before = snapshot(&root);
    let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .arg("--root")
        .arg(&root)
        .args([
            "knowledge",
            "search",
            "Synthetic",
            "--scope",
            "all",
            "--limit",
            "5",
        ])
        .env("TALKDND_NO_PDF2MD", "1")
        .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("id:"), "{text}");
    assert!(text.contains("authority:"), "{text}");
    assert!(!text.contains(&root.display().to_string()), "{text}");
    for (args, wanted) in [
        (vec!["knowledge", "get", "save:entity:E-001"], "id:"),
        (
            vec!["knowledge", "cite", "ruling:common:R-01"],
            "Rules/rulings.md",
        ),
        (vec!["knowledge", "context", "E-001"], "当前场景"),
        (
            vec!["knowledge", "related", "save:entity:E-001"],
            "authority:",
        ),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
            .arg("--root")
            .arg(&root)
            .args(&args)
            .env("TALKDND_NO_PDF2MD", "1")
            .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{args:?}: {text}");
        assert!(text.contains(wanted), "{args:?}: {text}");
        assert!(
            !text.contains(&root.display().to_string()),
            "{args:?}: {text}"
        );
    }
    // A query/key that is itself an unrevealed secret is not echoed back.
    for args in [
        vec!["knowledge", "search", "TOP-SECRET-DMSECRET"],
        vec!["knowledge", "context", "TOP-SECRET-DMSECRET"],
        vec!["knowledge", "get", "TOP-SECRET-DMSECRET"],
        vec!["knowledge", "cite", "TOP-SECRET-DMSECRET"],
        vec!["knowledge", "related", "TOP-SECRET-DMSECRET"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
            .arg("--root")
            .arg(&root)
            .args(&args)
            .env("TALKDND_NO_PDF2MD", "1")
            .env("TALKDND_COMBAT_SIM", root.join("no-such-combat-sim"))
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(
            !text.contains("TOP-SECRET-DMSECRET"),
            "{args:?} echoed the secret: {text}"
        );
        if args[1] != "search" {
            assert!(!out.status.success(), "{args:?}: {text}");
        }
    }
    assert_eq!(before, snapshot(&root), "knowledge must not write");

    let help = Command::new(env!("CARGO_BIN_EXE_talkdnd"))
        .args(["knowledge", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&help.stdout).to_string();
    for cmd in ["search", "get", "cite", "context", "related"] {
        assert!(text.contains(cmd), "{text}");
    }
}
