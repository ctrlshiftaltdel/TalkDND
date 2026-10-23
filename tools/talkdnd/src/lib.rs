//! TalkDND play helper. Mechanical, non-LLM steps for the AI DM:
//! resume digest, new save skeleton, character creation, random companions,
//! checkpoints and log entries. All dice go through combat-sim's engine.

pub mod adventure;
pub mod audit;
pub mod chargen;
pub mod charsheet;
pub mod check;
pub mod classdata;
pub mod data;
pub mod handout;
pub mod identify;
pub mod known;
pub mod migrate;
pub mod npc;
pub mod options;
pub mod original;
pub mod progress;
pub mod reveal;
pub mod rolls;
pub mod save;
pub mod secret;
pub mod settings;
pub mod start;
pub mod update;

use anyhow::{bail, Context, Result};
use chargen::Creation;
use clap::{Parser, Subcommand};
use combat_sim::dice::{seeded_rng, step_rng, DiceRng};
use combat_sim::fsutil::atomic_write;
use std::ffi::OsString;
use std::fs;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "talkdnd",
    version,
    about = "TalkDND play helper (finds its play folder from bin/.., TALKDND_ROOT or --root)"
)]
struct Cli {
    /// Play folder (default: TALKDND_ROOT, else the folder above bin/, else found from the current directory)
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    /// Save slot (default: newest)
    #[arg(long, global = true)]
    save: Option<String>,
    /// Reproducible dice for tests only. Never use in play.
    #[arg(long, global = true, hide = true)]
    seed: Option<u64>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Clone)]
enum Cmd {
    /// Resume: content check, background pdf2md, write and print Saves/<slot>/digest.md
    Start {
        #[arg(long)]
        no_pdf2md: bool,
    },
    /// Create a new save skeleton and start character creation.
    /// The deviation scope (0-2) and ratio (5-25%) are locked when the save is
    /// created; ask the player first. Without flags in a terminal, asks here.
    New {
        #[arg(long)]
        ruleset: Option<String>,
        #[arg(long)]
        adventure: Option<String>,
        /// Slot date (default: today, local time)
        #[arg(long)]
        date: Option<String>,
        /// 剧本偏离度：0 = 只补空白，1 = 次要原创，2 = 完整支线（开档后锁定）
        #[arg(long)]
        scope: Option<u32>,
        /// Original-content ceiling in percent (5-25; required for scope 1/2)
        #[arg(long)]
        ratio: Option<u32>,
    },
    /// Migrate an old save's on-disk schema (backs the whole save up first)
    Migrate {
        /// Only report what would be migrated; write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Character creation (player decides every step)
    Chargen {
        #[command(subcommand)]
        cmd: ChargenCmd,
    },
    /// Companion NPCs
    Npc {
        #[command(subcommand)]
        cmd: NpcCmd,
    },
    /// Ability/skill check or saving throw outside combat (modifier from the sheet; logged).
    /// Without arguments: save audit (integrity manifest + log reconciliation)
    Check {
        /// Character id or name (Characters/ or NPCs/)
        #[arg(requires = "what")]
        who: Option<String>,
        /// Skill id (perception), ability (wis), save:<ability> or init
        what: Option<String>,
        #[arg(long)]
        dc: Option<i32>,
        #[arg(long)]
        adv: bool,
        #[arg(long)]
        dis: bool,
        /// Extra dice or bonus, e.g. 1d4 (guidance) or +2
        #[arg(long, allow_hyphen_values = true)]
        extra: Option<String>,
    },
    /// Append a `## 存档点` section to the newest session log (text from args or stdin)
    Checkpoint { text: Vec<String> },
    /// Append one timestamped line to the newest session log
    Log {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        text: Vec<String>,
    },
    /// Local rulebook data (non-SRD classes, subclasses, weapons; gitignored)
    Data {
        #[command(subcommand)]
        cmd: DataCmd,
    },
    /// Experience points (ledger in Saves/<slot>/progress.yaml)
    Xp {
        #[command(subcommand)]
        cmd: XpCmd,
    },
    /// Level a character up: edits the sheet's combat block (level, HP, proficiency, hit dice)
    LevelUp {
        /// Character id or name
        who: String,
        /// HP gain: avg (default), roll, or the die result the player rolled
        #[arg(long, default_value = "avg")]
        hp: String,
        /// Class id when the sheet does not say (e.g. rogue)
        #[arg(long)]
        class: Option<String>,
        /// Milestone level for a major story beat: no XP check; XP is raised to the new level's threshold
        #[arg(long, requires = "reason")]
        milestone: bool,
        /// The story event (required with --milestone; one level per event per character)
        #[arg(long)]
        reason: Option<String>,
    },
    /// Coins and items per character and the shared `party` pool
    Loot {
        #[command(subcommand)]
        cmd: LootCmd,
    },
    /// Save integrity manifest: status, or accept hand edits with a reason
    Integrity {
        #[command(subcommand)]
        cmd: IntegrityCmd,
    },
    /// Original content ledger for this save (DM only): register an original
    /// element before introducing it, list them, or show the budget
    Original {
        #[command(subcommand)]
        cmd: OriginalCmd,
    },
    /// Read one scene of the locally imported adventure (never the whole book)
    Adventure {
        #[command(subcommand)]
        cmd: AdventureCmd,
    },
    /// What the players currently know (facts, handouts, identified names,
    /// met original content, party, chapter/location)
    Known,
    /// Record the current scene/location and print `known`
    Scene { name: Vec<String> },
    /// Secret terms for the spoiler guard (DM only, stored in the save)
    Secret {
        #[command(subcommand)]
        cmd: SecretCmd,
    },
    /// Check a draft narration/option list for unrevealed secret terms
    /// (text from args or stdin; exit code 1 when anything is flagged)
    Guard {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        text: Vec<String>,
    },
    /// Record that the players identified a monster/item, or set the
    /// player-facing appearance used until they do
    Identify {
        /// Creature/item name or id
        thing: Option<String>,
        /// How they learned it: check (investigation/perception), reveal
        /// (magic/truth) or lore (research/hearsay)
        #[arg(long, default_value = "reveal", value_parser = ["check", "reveal", "lore"])]
        how: String,
        #[arg(long)]
        reason: Option<String>,
        /// Override the player-facing description: --appearance <thing> "<描述>"
        #[arg(long, num_args = 2, value_names = ["THING", "DESCRIPTION"])]
        appearance: Option<Vec<String>>,
    },
    /// Table settings (dice display) and `log rolls`
    Settings {
        #[command(subcommand)]
        cmd: Option<SettingsCmd>,
    },
    /// Revealed-knowledge ledger (what the party has actually learned)
    Reveal {
        #[command(subcommand)]
        cmd: RevealCmd,
    },
    /// Book images shown to the players (maps, letters, items). Only
    /// player-safe entries are exported; DM-only maps never are
    Handout {
        /// Converted book folder or book/adventure name (default: the save's adventure)
        #[arg(long, global = true)]
        book: Option<String>,
        #[command(subcommand)]
        cmd: HandoutCmd,
    },
    /// Character sheet changes (HP, resources, rests, conditions, items, gold,
    /// choices, notes). The sheet is rendered from Saves/<slot>/sheets/<id>.yaml;
    /// never hand-edit it. `talkdnd char Yier batch "damage 7; use slot1"`
    Char {
        /// Character id or name (`all` for `rest`)
        who: String,
        #[command(subcommand)]
        op: Option<charsheet::Op>,
    },
    /// List SRD options for a ruleset
    Options {
        #[arg(long, default_value = "2024")]
        ruleset: String,
    },
}

#[derive(Subcommand, Clone)]
enum IntegrityCmd {
    /// Files changed outside the tools and unexplained value changes
    Status,
    /// Accept the current files and values (logged with the reason)
    Accept {
        #[arg(long)]
        reason: String,
    },
}

#[derive(Subcommand, Clone)]
enum HandoutCmd {
    /// Export a player-safe handout to Saves/<slot>/handouts/, print its path
    /// and a Markdown image link, and record it (reveal ledger, session log)
    Show {
        /// Handout id (p0012-00) or a caption / area search
        query: Vec<String>,
        /// Only this region: x,y,w,h as fractions 0-1 or pixels (exported as PNG)
        #[arg(long)]
        crop: Option<String>,
        /// Convert to PNG (for hosts that do not display WebP)
        #[arg(long)]
        png: bool,
    },
    /// Handouts shown so far; --dm lists the whole registry with visibility
    List {
        #[arg(long)]
        dm: bool,
    },
    /// Build or refresh the registry from the pdf2md index (keeps markings)
    Build,
    /// Mark a handout player-safe (--player) or DM-only (--dm)
    Mark {
        id: String,
        #[arg(long, conflicts_with = "dm")]
        player: bool,
        #[arg(long)]
        dm: bool,
        /// Player-safe region x,y,w,h (only this part is ever exported; "" clears)
        #[arg(long)]
        crop: Option<String>,
        #[arg(long)]
        caption: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
}

#[derive(Subcommand, Clone)]
enum OriginalCmd {
    /// Register an original element before it appears in play (DM only)
    Add {
        /// encounter | npc | item | location | sidequest
        #[arg(long = "type")]
        kind: String,
        #[arg(long)]
        name: String,
        /// common | uncommon | rare | very-rare | legendary (required for item)
        #[arg(long)]
        rarity: Option<String>,
        #[arg(long, default_value = "")]
        note: String,
        /// DM-only hidden aspect (secret term source until revealed)
        #[arg(long, default_value = "")]
        hidden: String,
        /// The idea came from a player: `--hook player` (adopted player hooks
        /// count as original)
        #[arg(long, value_name = "SOURCE", value_parser = ["player"])]
        hook: Option<String>,
    },
    /// List this save's original-content ledger (DM only)
    List,
    /// Show the deviation settings, caps and the current chapter budget
    Status,
    /// Create Saves/<slot>/dm-plan.md (DM-only long-term plan template)
    Plan,
}

#[derive(Subcommand, Clone)]
enum AdventureCmd {
    /// Print one heading-delimited section; `--list` lists headings only
    Section {
        /// Heading, chapter or area search
        query: Vec<String>,
        /// List section headings only (no body text)
        #[arg(long)]
        list: bool,
        /// Read a section beyond progress anyway (logged)
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Subcommand, Clone)]
enum SecretCmd {
    /// Add a manual secret term (stored only in this save)
    Add {
        term: Vec<String>,
        /// Chapter number this secret belongs to (optional)
        #[arg(long)]
        chapter: Option<u32>,
        #[arg(long, default_value = "")]
        note: String,
    },
    /// List every unrevealed secret term and its source
    List,
    /// Remove a manual secret term
    Remove { term: Vec<String> },
}

#[derive(Subcommand, Clone)]
enum SettingsCmd {
    /// Show or change dice display: `settings dice on|off`
    Dice { value: Option<String> },
}

#[derive(Subcommand, Clone)]
enum RevealCmd {
    /// Record a fact the party has learned (also logged)
    Add {
        fact: Vec<String>,
        /// How they learned it (check, NPC, document, rumour)
        #[arg(long, default_value = "")]
        source: String,
        /// Tags such as npc:<id>, place:<id> (repeatable)
        #[arg(long)]
        tag: Vec<String>,
        /// Mark the hidden aspect of this original entry as revealed
        /// (`reveal add --original O-001`; fact is optional then)
        #[arg(long)]
        original: Option<String>,
    },
    /// List revealed facts
    List {
        #[arg(long)]
        tag: Option<String>,
    },
    /// Is anything about <term> revealed? Run before naming it in narration
    Check { term: Vec<String> },
}

#[derive(Subcommand, Clone)]
enum XpCmd {
    /// Award XP for a small quest or event, split evenly:
    /// `xp add 450 --category combat --reason "museum guards"` or
    /// `xp add --difficulty hard --category noncombat --reason "baron talks"`
    Add {
        /// Total XP (omit with --difficulty)
        amount: Option<i64>,
        /// Amount from the ruleset table: 2014 easy|medium|hard|deadly, 2024 low|moderate|high
        #[arg(long)]
        difficulty: Option<String>,
        /// combat, noncombat, quest, discovery, roleplay, milestone-minor
        #[arg(long, default_value = "combat")]
        category: String,
        /// `all` (campaign party) or comma-separated names/ids
        #[arg(long, default_value = "all")]
        to: String,
        /// What happened (required; the same reason is not awarded twice)
        #[arg(long)]
        reason: String,
        /// Allow a second award with the same reason
        #[arg(long)]
        again: bool,
    },
    /// XP, level and next threshold per character
    Show,
}

#[derive(clap::Args, Clone)]
struct Coins {
    #[arg(long, default_value_t = 0)]
    pp: i64,
    #[arg(long, default_value_t = 0)]
    gp: i64,
    #[arg(long, default_value_t = 0)]
    ep: i64,
    #[arg(long, default_value_t = 0)]
    sp: i64,
    #[arg(long, default_value_t = 0)]
    cp: i64,
}

impl Coins {
    fn total(&self) -> Result<i64> {
        let t = progress::to_cp(self.pp, self.gp, self.ep, self.sp, self.cp);
        if [self.pp, self.gp, self.ep, self.sp, self.cp]
            .iter()
            .any(|v| *v < 0)
        {
            bail!("coin amounts must be 0 or more");
        }
        Ok(t)
    }
}

#[derive(Subcommand, Clone)]
enum LootCmd {
    /// Add coins/items: `loot add --gp 50 "potion of healing" --to party`
    Add {
        items: Vec<String>,
        #[command(flatten)]
        coins: Coins,
        /// `party` (shared pool) or a character
        #[arg(long, default_value = "party")]
        to: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Spend or lose coins/items
    Spend {
        items: Vec<String>,
        #[command(flatten)]
        coins: Coins,
        #[arg(long, default_value = "party")]
        from: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Move coins/items between purses
    Give {
        items: Vec<String>,
        #[command(flatten)]
        coins: Coins,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
    /// Split the party pool's coins evenly among the party
    Split,
    /// Show every purse
    Show,
}

#[derive(Subcommand, Clone)]
enum ChargenCmd {
    /// Show choices so far and the next step
    Status,
    /// Roll 6 x 4d6dl1 (logged); the player then assigns them
    Roll,
    /// Use the standard array 15 14 13 12 10 8
    Array,
    /// Point buy: e.g. `str=15 dex=14 con=13 int=10 wis=10 cha=8` (27 points)
    Pointbuy { scores: Vec<String> },
    /// Player assigns pool numbers: `str=15 dex=14 ...`
    Assign { scores: Vec<String> },
    /// Player-chosen increases: `str=2 con=1`, or `none`
    Asi { values: Vec<String> },
    /// Record a choice: ruleset|adventure|species|class|level|subclass|fighting-style|masteries|background|background-equipment|skills|background-skills|languages|tools|feats|class-equipment|armor|shield|equipment|spells|name|traits|ideals|bonds|flaws|alignment
    Set { field: String, value: Vec<String> },
    /// Check the character against the ruleset
    Validate,
    /// Write Characters/<name>.md (sheet + combat yaml in sync)
    Sheet {
        #[arg(long, default_value = "玩家")]
        player: String,
    },
    /// Player confirmed the sheet: mark creation complete
    Confirm,
}

#[derive(Subcommand, Clone)]
enum DataCmd {
    /// Show which local data files are loaded and whether converted Markdown exists
    Status {
        #[arg(long, default_value = "2024")]
        ruleset: String,
    },
    /// Build Rules/chargen/local/imported-<ruleset>.yaml from converted Markdown (.md/)
    Import {
        #[arg(long, default_value = "2024")]
        ruleset: String,
    },
}

#[derive(Subcommand, Clone)]
enum NpcCmd {
    /// Generate random SRD companions (rolls logged when a save exists); DM adds name/backstory
    Random {
        #[arg(long)]
        ruleset: Option<String>,
        /// Default: the PC's level from this save's character creation (else 1).
        #[arg(long)]
        level: Option<i32>,
        #[arg(long, default_value_t = 1)]
        count: usize,
    },
    /// Write NPCs/<name>.md from draft N (after the player confirms) and add to party
    Sheet {
        draft: usize,
        #[arg(long)]
        name: String,
        #[arg(long)]
        personality: Option<String>,
    },
}

pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> Result<()> {
    let cli = Cli::parse_from(args);
    let root = save::find_root(cli.root.as_deref())?;
    let mut os = step_rng();
    let mut seeded = cli.seed.map(seeded_rng);
    let rng: &mut dyn DiceRng = match seeded.as_mut() {
        Some(r) => r,
        None => &mut os,
    };
    let tracked = match &cli.cmd {
        Cmd::Start { .. }
        | Cmd::New { .. }
        | Cmd::Migrate { .. }
        | Cmd::Integrity { .. }
        | Cmd::Options { .. }
        | Cmd::Data { .. } => None,
        _ => save::resolve_slot(&root, cli.save.as_deref()).ok(),
    };
    let before = tracked.as_deref().map(combat_sim::integrity::snapshot);
    let is_new = matches!(cli.cmd, Cmd::New { .. });
    let result = dispatch(&root, &cli, rng);
    if let (Some(s), Some(before)) = (tracked.as_deref(), before.as_ref()) {
        if s.is_dir() {
            audit::after_command(s, before)?;
        }
    }
    if is_new && result.is_ok() {
        if let Ok(s) = save::resolve_slot(&root, None) {
            audit::audit(&s)?;
        }
    }
    result
}

fn print_audit(s: &Path) -> Result<()> {
    let r = audit::audit(s)?;
    if r.created {
        println!(
            "integrity: no manifest yet — created a baseline from the current files (old save)"
        );
    }
    if r.clean() {
        println!("integrity: OK — files match the last tool writes; value changes are all logged");
    } else {
        for l in r.lines() {
            println!("warning: {l}");
        }
    }
    Ok(())
}

fn dispatch(root: &Path, cli: &Cli, rng: &mut dyn DiceRng) -> Result<()> {
    let root = root.to_path_buf();
    match cli.cmd.clone() {
        Cmd::Char { who, op } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            for l in charsheet::run(&s, &who, op.unwrap_or(charsheet::Op::Show), rng)? {
                println!("{l}");
            }
        }
        Cmd::Integrity { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            match cmd {
                IntegrityCmd::Status => print_audit(&s)?,
                IntegrityCmd::Accept { reason } => {
                    if reason.trim().is_empty() {
                        bail!("--reason is required");
                    }
                    for l in audit::accept(&s, &reason)? {
                        println!("{l}");
                    }
                }
            }
        }
        Cmd::Original { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let lines = match cmd {
                OriginalCmd::Add {
                    kind,
                    name,
                    rarity,
                    note,
                    hidden,
                    hook,
                } => original::add(
                    &s,
                    &original::New {
                        kind: &kind,
                        name: &name,
                        rarity: rarity.as_deref(),
                        note: &note,
                        hidden: &hidden,
                        player_hook: hook.is_some(),
                    },
                )?,
                OriginalCmd::List => original::list(&s)?,
                OriginalCmd::Status => original::status_lines(&s)?,
                OriginalCmd::Plan => vec![original::plan(&s)?],
            };
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::Adventure { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let lines = match cmd {
                AdventureCmd::Section {
                    query,
                    list,
                    reason,
                } => adventure::section(&root, &s, &query.join(" "), list, reason.as_deref())?,
            };
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::Known => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            for l in known::lines(&s, false) {
                println!("{l}");
            }
        }
        Cmd::Scene { name } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            for l in known::scene_change(&s, &name.join(" "))? {
                println!("{l}");
            }
        }
        Cmd::Secret { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let lines = match cmd {
                SecretCmd::Add {
                    term,
                    chapter,
                    note,
                } => vec![secret::add(&s, &term.join(" "), chapter, &note)?],
                SecretCmd::List => secret::list_lines(&s, &root),
                SecretCmd::Remove { term } => vec![secret::remove(&s, &term.join(" "))?],
            };
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::Guard { text } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let mut draft = text.join(" ");
            if draft.trim().is_empty() || draft.trim() == "-" {
                draft.clear();
                std::io::stdin().read_to_string(&mut draft)?;
            }
            if draft.trim().is_empty() {
                bail!("guard needs text as arguments or on stdin");
            }
            let hits = secret::guard_terms(&s, &root, &draft);
            if hits.is_empty() {
                println!("guard: clean（没有发现未揭示的秘密词）");
            } else {
                for h in &hits {
                    println!("{h}");
                }
                println!(
                    "guard: {} 个未揭示秘密词；改写为玩家已知的说法后再叙述",
                    hits.len()
                );
                std::process::exit(1);
            }
        }
        Cmd::Identify {
            thing,
            how,
            reason,
            appearance,
        } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let line = if let Some(pair) = appearance {
                if pair.len() != 2 {
                    bail!("--appearance needs <thing> \"<description>\"");
                }
                identify::set_appearance(&s, &pair[0], &pair[1])?
            } else {
                let thing = thing
                    .filter(|t| !t.trim().is_empty())
                    .ok_or_else(|| anyhow::anyhow!("identify needs a name or --appearance"))?;
                identify::identify(&s, &thing, &how, reason.as_deref())?
            };
            println!("{line}");
        }
        Cmd::Settings { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            match cmd {
                None => {
                    for l in settings::lines(&s) {
                        println!("{l}");
                    }
                }
                Some(SettingsCmd::Dice { value }) => {
                    match settings::parse_dice(value.as_deref())? {
                        None => {
                            for l in settings::lines(&s) {
                                println!("{l}");
                            }
                        }
                        Some(on) => {
                            for l in settings::set_dice(&root, &s, on, None)? {
                                println!("{l}");
                            }
                        }
                    }
                }
            }
        }
        Cmd::Reveal { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let lines = match cmd {
                RevealCmd::Add {
                    fact,
                    source,
                    tag,
                    original,
                } => {
                    if let Some(id) = original {
                        vec![reveal::add_original(
                            &s,
                            &id,
                            Some(&fact.join(" ")),
                            &source,
                        )?]
                    } else {
                        vec![reveal::add(&s, &fact.join(" "), &source, &tag)?]
                    }
                }
                RevealCmd::List { tag } => reveal::list(&s, tag.as_deref())?,
                RevealCmd::Check { term } => reveal::check(&s, &term.join(" "))?,
            };
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::Handout { book, cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let b = book.as_deref();
            let lines = match cmd {
                HandoutCmd::Show { query, crop, png } => handout::show(
                    &root,
                    &s,
                    b,
                    &query.join(" "),
                    &handout::ShowOpts {
                        crop: crop.as_deref(),
                        png,
                    },
                )?,
                HandoutCmd::List { dm } => handout::list(&root, &s, b, dm)?,
                HandoutCmd::Build => handout::rebuild(&root, &s, b)?,
                HandoutCmd::Mark {
                    id,
                    player,
                    dm,
                    crop,
                    caption,
                    note,
                } => {
                    if !player && !dm {
                        bail!("--player or --dm");
                    }
                    vec![handout::mark(
                        &root,
                        &s,
                        b,
                        &id,
                        &handout::Mark {
                            player,
                            crop: crop.as_deref(),
                            caption: caption.as_deref(),
                            note: note.as_deref(),
                        },
                    )?]
                }
            };
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::Start { no_pdf2md } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let d = start::digest(&root, &s, &start::Options { pdf2md: !no_pdf2md })?;
            print!("{d}");
        }
        Cmd::Migrate { dry_run } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let report = migrate::ensure(&s, &root, dry_run)?;
            for line in report.lines() {
                println!("{line}");
            }
        }
        Cmd::New {
            ruleset,
            adventure,
            date,
            scope,
            ratio,
        } => new_save(&root, ruleset, adventure, date, scope, ratio)?,
        Cmd::Chargen { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            chargen_cmd(&s, cmd, rng, cli.seed.is_some())?
        }
        Cmd::Npc { cmd } => npc_cmd(&root, cli.save.as_deref(), cmd, rng, cli.seed.is_some())?,
        Cmd::Check {
            who,
            what,
            dc,
            adv,
            dis,
            extra,
        } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let (Some(who), Some(what)) = (who, what) else {
                return print_audit(&s);
            };
            let r = audit::audit(&s)?;
            if !r.clean() {
                eprintln!("warning: save audit found {} issue(s); run `talkdnd check` (no arguments) for details", r.issue_count());
            }
            let line = check::run(
                &s,
                &who,
                &what,
                dc,
                adv,
                dis,
                extra.as_deref(),
                rng,
                cli.seed.is_some(),
            )?;
            println!("{line}");
        }
        Cmd::Xp { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let lines = match cmd {
                XpCmd::Add {
                    amount,
                    difficulty,
                    category,
                    to,
                    reason,
                    again,
                } => progress::xp_add(
                    &s,
                    &progress::Award {
                        amount,
                        difficulty: difficulty.as_deref(),
                        category: &category,
                        to: &to,
                        reason: &reason,
                        again,
                    },
                )?,
                XpCmd::Show => progress::xp_show(&s)?,
            };
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::LevelUp {
            who,
            hp,
            class,
            milestone,
            reason,
        } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let ms = if milestone { reason.as_deref() } else { None };
            let whos: Vec<String> = if who == "all" {
                start::party_ids(&s)
            } else {
                who.split(',').map(|w| w.trim().to_string()).collect()
            };
            for w in whos {
                let r = progress::level_up(
                    &s,
                    &root,
                    &w,
                    &hp,
                    class.as_deref(),
                    ms,
                    rng,
                    cli.seed.is_some(),
                )?;
                for l in r.lines {
                    println!("{l}");
                }
            }
        }
        Cmd::Loot { cmd } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let lines = match &cmd {
                LootCmd::Add {
                    items,
                    coins,
                    to,
                    reason,
                } => progress::loot(
                    &s,
                    progress::LootOp::Add {
                        to,
                        cp: coins.total()?,
                        items,
                    },
                    reason.as_deref(),
                )?,
                LootCmd::Spend {
                    items,
                    coins,
                    from,
                    reason,
                } => progress::loot(
                    &s,
                    progress::LootOp::Spend {
                        from,
                        cp: coins.total()?,
                        items,
                    },
                    reason.as_deref(),
                )?,
                LootCmd::Give {
                    items,
                    coins,
                    from,
                    to,
                } => progress::loot(
                    &s,
                    progress::LootOp::Give {
                        from,
                        to,
                        cp: coins.total()?,
                        items,
                    },
                    None,
                )?,
                LootCmd::Split => progress::loot(&s, progress::LootOp::Split, None)?,
                LootCmd::Show => progress::loot_show(&s)?,
            };
            for l in lines {
                println!("{l}");
            }
        }
        Cmd::Checkpoint { text } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let mut body = text.join(" ");
            if body.trim().is_empty() {
                std::io::stdin().read_to_string(&mut body)?;
            }
            if body.trim().is_empty() {
                bail!("checkpoint text is empty (pass it as arguments or on stdin)");
            }
            let log = save::log_path(&s)?;
            let now = chrono::Local::now().format("%Y-%m-%d %H:%M");
            save::append(&log, &format!("\n## 存档点（{now}）\n\n{}\n", body.trim()))?;
            println!("checkpoint appended to {}", rel(&root, &log));
        }
        Cmd::Log { text } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            if text.first().is_some_and(|t| t == "rolls") {
                let mut last = None;
                let mut args = text.iter().skip(1);
                while let Some(a) = args.next() {
                    match a.as_str() {
                        "--last" | "-n" => {
                            let n = args
                                .next()
                                .ok_or_else(|| anyhow::anyhow!("--last needs a number"))?;
                            last = Some(n.parse::<usize>().context("--last must be a number")?);
                        }
                        other => bail!("unknown `log rolls` option `{other}`"),
                    }
                }
                for l in rolls::show(&s, last) {
                    println!("{l}");
                }
                return Ok(());
            }
            let body = text.join(" ");
            if body.trim().is_empty() {
                bail!("log text is empty");
            }
            let log = save::log_path(&s)?;
            let now = chrono::Local::now().format("%H:%M");
            save::append(&log, &format!("- [{now}] {}", body.trim()))?;
            println!("logged to {}", rel(&root, &log));
        }
        Cmd::Options { ruleset } => print_options(&ruleset),
        Cmd::Data { cmd } => data_cmd(&root, cmd)?,
    }
    Ok(())
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn check_ruleset(r: &str) -> Result<()> {
    if r != "2014" && r != "2024" {
        bail!("ruleset must be 2014 or 2024");
    }
    Ok(())
}

fn deviation_input(scope: Option<u32>, ratio: Option<u32>) -> Result<migrate::Deviation> {
    let interactive = std::io::stdin().is_terminal();
    if !interactive {
        if scope.is_none() {
            let d = migrate::Deviation::new(Some(0), ratio)?;
            println!("剧本偏离度：未指定（非交互），使用默认 {}", d.describe());
            return Ok(d);
        }
        return migrate::Deviation::new(scope, ratio);
    }
    // Interactive: ask for whatever is missing, one question at a time.
    let scope = match scope {
        Some(s) => s,
        None => {
            println!("剧本偏离度（开档后锁定，不能修改）：");
            println!("  0 = 不偏离：只补必要空白（给未命名 NPC 起名、对白措辞、场景描述）");
            println!("  1 = 次要原创：遭遇、NPC、物品、地点；稀有度上限 uncommon");
            println!("  2 = 完整支线：在 1 的基础上可加支线；稀有度上限 rare");
            println!("  3 及以上暂未开放。");
            prompt_scope()?
        }
    };
    if scope > 2 || scope == 0 {
        return migrate::Deviation::new(Some(scope), ratio);
    }
    let ratio = match ratio {
        Some(r) => r,
        None => prompt_ratio()?,
    };
    migrate::Deviation::new(Some(scope), Some(ratio))
}

fn prompt_scope() -> Result<u32> {
    use std::io::Write;
    loop {
        print!("选择 scope（0/1/2）：");
        std::io::stdout().flush()?;
        let line = read_prompt_line()?;
        match line.trim().parse::<u32>() {
            Ok(n) if n <= 2 => return Ok(n),
            Ok(n) => println!("scope {n} 暂未开放（scope 3 及以上不支持），请输入 0、1 或 2"),
            Err(_) => println!("请输入 0、1 或 2"),
        }
    }
}

fn prompt_ratio() -> Result<u32> {
    use std::io::Write;
    loop {
        print!(
            "原创比例上限（{} 到 {} 的整数百分比，例如 15）：",
            migrate::Deviation::MIN_RATIO,
            migrate::Deviation::MAX_RATIO
        );
        std::io::stdout().flush()?;
        let line = read_prompt_line()?;
        match line.trim().parse::<u32>() {
            Ok(n)
                if (migrate::Deviation::MIN_RATIO..=migrate::Deviation::MAX_RATIO).contains(&n) =>
            {
                return Ok(n)
            }
            _ => println!(
                "必须是 {} 到 {} 的整数",
                migrate::Deviation::MIN_RATIO,
                migrate::Deviation::MAX_RATIO
            ),
        }
    }
}

fn read_prompt_line() -> Result<String> {
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line)? == 0 {
        bail!("没有读到输入；非交互环境请用 --scope 和 --ratio");
    }
    Ok(line)
}

fn new_save(
    root: &Path,
    ruleset: Option<String>,
    adventure: Option<String>,
    date: Option<String>,
    scope: Option<u32>,
    ratio: Option<u32>,
) -> Result<()> {
    if let Some(r) = &ruleset {
        check_ruleset(r)?;
    }
    let deviation = deviation_input(scope, ratio)?;
    let date = date.unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    if !save::is_slot_name(&format!("{date}-01")) {
        bail!("date must be YYYY-MM-DD");
    }
    let slot = save::next_slot_name(root, &date)?;
    let dir = root.join("Saves").join(&slot);
    for d in ["Characters", "NPCs", "Sessions", "combat"] {
        fs::create_dir_all(dir.join(d))?;
    }
    let rs = ruleset.clone().unwrap_or_else(|| "2024".into());
    let adv_slug = adventure.as_deref().map(chargen::slug);
    let adv_line = match &adv_slug {
        Some(a) => {
            fs::create_dir_all(root.join("Adventures").join(a).join("combat"))?;
            format!("adventures: [../../../Adventures/{a}/combat]\n")
        }
        None => "adventures: []\n".into(),
    };
    let campaign = format!(
        "# combat-sim campaign manifest for save {slot} (paths relative to this file)\nname: \"TalkDND {slot}\"\nlibrary: [../../../Rules/combat]\n{adv_line}characters: [../Characters, ../NPCs]\nstate_dir: .\nparty: []\nruleset: \"{rs}\"   # 2014 | 2024\n"
    );
    atomic_write(dir.join("combat/campaign.yaml"), campaign.as_bytes())?;
    atomic_write(dir.join("canon.md"), "# 即兴定论（公开）\n\n".as_bytes())?;
    atomic_write(
        dir.join("dm-secrets.md"),
        "# DM 秘密（只给 DM）\n\n".as_bytes(),
    )?;
    let setup = format!(
        "# {slot} 第 0 节：建卡\n\n- [{}] 新存档，规则 {}\n- 剧本偏离度：{}（开档后锁定）\n",
        chrono::Local::now().format("%H:%M"),
        ruleset.as_deref().unwrap_or("待定"),
        deviation.describe()
    );
    atomic_write(
        dir.join(format!("Sessions/{date}-session-00-setup.md")),
        setup.as_bytes(),
    )?;
    let c = Creation {
        ruleset,
        adventure,
        level: 1,
        ..Default::default()
    };
    chargen::store(&dir, &c)?;
    migrate::init_with_deviation(&dir, &deviation)?;
    println!("created Saves/{slot}");
    println!(
        "剧本偏离度：{}（开档后锁定，不能修改）",
        deviation.describe()
    );
    for line in update::sync_defaults(root)? {
        println!("{line}");
    }
    print_next(&c);
    Ok(())
}

fn print_next(c: &Creation) {
    match c.next_step() {
        Some((step, label)) => println!(
            "next step: {step} — {label}  (ask the player, then record it; do not choose for them)"
        ),
        None => println!("character creation complete"),
    }
}

fn set_campaign_line(save: &Path, key: &str, value: &str) -> Result<()> {
    let p = save.join("combat/campaign.yaml");
    let text = fs::read_to_string(&p)?;
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            if l.starts_with(&format!("{key}:")) {
                found = true;
                format!("{key}: {value}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        out.push(format!("{key}: {value}"));
    }
    atomic_write(&p, (out.join("\n") + "\n").as_bytes())?;
    Ok(())
}

fn add_to_party(save: &Path, id: &str) -> Result<()> {
    let p = save.join("combat/campaign.yaml");
    let text = fs::read_to_string(&p).unwrap_or_default();
    let line = text
        .lines()
        .find(|l| l.starts_with("party:"))
        .unwrap_or("party: []");
    let inner = line["party:".len()..]
        .split('#')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['[', ']']);
    let mut ids: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if !ids.iter().any(|x| x == id) {
        ids.push(id.to_string());
    }
    set_campaign_line(save, "party", &format!("[{}]", ids.join(", ")))
}

fn log_rolls(save: &Path, who: &str, texts: &[String], seeded: bool) -> Result<()> {
    if let Some(log) = save::latest_log(save) {
        let tag = if seeded { " (seeded test roll)" } else { "" };
        save::append(
            &log,
            &format!(
                "- [{}] {who} 属性掷骰 6×4d6dl1{tag}：{}",
                chrono::Local::now().format("%H:%M"),
                texts.join("  ")
            ),
        )?;
    }
    Ok(())
}

fn chargen_cmd(save: &Path, cmd: ChargenCmd, rng: &mut dyn DiceRng, seeded: bool) -> Result<()> {
    let mut c = chargen::load(save)?;
    match cmd {
        ChargenCmd::Status => {
            for (step, label) in chargen::STEPS {
                println!(
                    "[{}] {step:<10} {label}",
                    if c.done(step) { "x" } else { " " }
                );
            }
            println!("{}", serde_json::to_string_pretty(&c)?);
            print_next(&c);
            return Ok(());
        }
        ChargenCmd::Roll => {
            if c.method.as_deref() == Some("roll") && !c.pool.is_empty() {
                bail!("already rolled {:?}; rolls are final (no rerolls)", c.pool);
            }
            let (pool, texts) = chargen::roll_pool(rng)?;
            log_rolls(save, "PC", &texts, seeded)?;
            rolls::append(
                save,
                "PC",
                "属性生成（6×4d6dl1）",
                &texts.join("  "),
                seeded,
            )?;
            for t in &texts {
                println!("{t}");
            }
            println!("pool: {pool:?} — the player assigns these with `chargen assign`");
            c.method = Some("roll".into());
            c.pool = pool;
            c.scores.clear();
        }
        ChargenCmd::Array => {
            if c.method.as_deref() == Some("roll") && !c.pool.is_empty() {
                bail!("scores were already rolled; rolls are final");
            }
            c.method = Some("array".into());
            c.pool = data::STANDARD_ARRAY.to_vec();
            c.scores.clear();
            println!(
                "pool: {:?} — the player assigns these with `chargen assign`",
                c.pool
            );
        }
        ChargenCmd::Pointbuy { scores } => {
            if c.method.as_deref() == Some("roll") && !c.pool.is_empty() {
                bail!("scores were already rolled; rolls are final");
            }
            c.method = Some("pointbuy".into());
            c.pool.clear();
            if !scores.is_empty() {
                let m = chargen::parse_abilities(&scores)?;
                if m.len() != 6 {
                    bail!("give all six abilities");
                }
                c.scores = m;
            }
        }
        ChargenCmd::Assign { scores } => {
            let m = chargen::parse_abilities(&scores)?;
            if m.len() != 6 {
                bail!("give all six abilities");
            }
            c.scores = m;
        }
        ChargenCmd::Asi { values } => {
            if values.len() == 1 && values[0] == "none" {
                c.asi.clear();
            } else {
                c.asi = chargen::parse_abilities(&values)?;
            }
            c.asi_done = true;
        }
        ChargenCmd::Set { field, value } => {
            let v = value.join(" ").trim().to_string();
            let list = || {
                v.split([',', ' '])
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            };
            match field.as_str() {
                "ruleset" => {
                    check_ruleset(&v)?;
                    set_campaign_line(save, "ruleset", &format!("\"{v}\"   # 2014 | 2024"))?;
                    c.ruleset = Some(v);
                }
                "adventure" => {
                    let s = chargen::slug(&v);
                    if let Some(root) = save.parent().and_then(|p| p.parent()) {
                        fs::create_dir_all(root.join("Adventures").join(&s).join("combat"))?;
                    }
                    set_campaign_line(
                        save,
                        "adventures",
                        &format!("[../../../Adventures/{s}/combat]"),
                    )?;
                    c.adventure = Some(v);
                }
                "species" | "race" => c.species = Some(chargen::slug(&v)),
                "class" => c.class = Some(chargen::slug(&v)),
                "subclass" => c.subclass = Some(chargen::slug(&v)),
                "masteries" => c.masteries = list().iter().map(|s| chargen::slug(s)).collect(),
                "background" => c.background = Some(chargen::slug(&v)),
                "skills" => c.skills = list(),
                "background-skills" => c.background_skills = list(),
                "armor" => c.armor = Some(chargen::slug(&v)),
                "shield" => c.shield = matches!(v.as_str(), "yes" | "true" | "1" | "y"),
                "equipment" => {
                    c.equipment = v
                        .split([',', ';'])
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                "spells" => {
                    c.spells = if v == "none" {
                        Vec::new()
                    } else {
                        v.split([',', ';'])
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    };
                    c.spells_done = true;
                }
                "name" => c.name = Some(v),
                "personality" | "traits" => c.personality = Some(v),
                "ideals" => c.ideals = Some(v),
                "bonds" => c.bonds = Some(v),
                "flaws" => c.flaws = Some(v),
                "alignment" => c.alignment = Some(v),
                "fighting-style" | "style" => {
                    let id = chargen::slug(&v);
                    match options::style(&id) {
                        Some(s) => println!("{}（{}）：{}", s.zh, s.name, s.effect),
                        None => {
                            println!("warning: `{id}` is not in the data; the DM rules its effect")
                        }
                    }
                    c.fighting_style = Some(id);
                }
                "feats" => {
                    c.feats = v
                        .split([',', ';'])
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                "languages" => {
                    c.languages = v
                        .split([',', ';'])
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty() && s != "none")
                        .collect();
                    c.proficiencies_done = true;
                }
                "tools" => {
                    c.tools = v
                        .split([',', ';'])
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty() && s != "none")
                        .collect();
                    c.proficiencies_done = true;
                }
                "class-equipment" => {
                    let cls = c.class.clone().context("set class first")?;
                    let r = options::class_equipment(c.rs(), &cls, &v, rng)?;
                    if let Some(t) = &r.roll {
                        println!("starting gold: {t}");
                        rolls::append(save, "PC", "起始金币", t, seeded)?;
                        if let Some(log) = save::latest_log(save) {
                            save::append(
                                &log,
                                &format!(
                                    "- [{}] PC 起始金币掷骰{}：{t}",
                                    chrono::Local::now().format("%H:%M"),
                                    if seeded { " (seeded test roll)" } else { "" }
                                ),
                            )?;
                        }
                    }
                    println!(
                        "class equipment: {}{}",
                        if r.items.is_empty() {
                            "—".into()
                        } else {
                            r.items.join(", ")
                        },
                        if r.gold > 0 {
                            format!(" + {} gp", r.gold)
                        } else {
                            String::new()
                        }
                    );
                    c.class_equipment = Some(v);
                    c.class_items = r.items;
                    c.class_gold = r.gold;
                }
                "background-equipment" => {
                    c.background_equipment = Some(v.to_uppercase());
                    let r = c.background_pack(
                        classdata::Book::load(c.rs(), save.parent().and_then(|p| p.parent()))
                            .ok()
                            .as_ref(),
                    );
                    println!(
                        "background equipment: {} + {} gp",
                        r.items.join(", "),
                        r.gold
                    );
                }
                "level" => {
                    c.level = v.parse().context("level must be a number")?;
                    c.level_done = true;
                }
                other => bail!("unknown field `{other}`"),
            }
        }
        ChargenCmd::Validate => {
            let r = chargen::validate(&c);
            report(&c, &r);
            if !r.errors.is_empty() {
                bail!("{} error(s)", r.errors.len());
            }
            return Ok(());
        }
        ChargenCmd::Sheet { player } => {
            let r = chargen::validate(&c);
            report(&c, &r);
            if !r.errors.is_empty() {
                bail!("fix {} error(s) before writing the sheet", r.errors.len());
            }
            let root = save.parent().and_then(|p| p.parent());
            let book = classdata::Book::load(c.rs(), root)?;
            let (id, md) = chargen::sheet_with(&c, &player, &book, root)?;
            for w in classdata::generate(&c, &book, root).warnings {
                println!("warning: {w}");
            }
            if !chargen::valid_id(&id) {
                bail!("character name must give an id of lowercase letters, digits and hyphens (got `{id}`)");
            }
            let file = save.join("Characters").join(format!("{id}.md"));
            atomic_write(&file, md.as_bytes())?;
            add_to_party(save, &id)?;
            charsheet::from_chargen(save, &c, &book, &file)?;
            println!("wrote Characters/{id}.md; show it to the player, then `chargen confirm`");
            return Ok(());
        }
        ChargenCmd::Confirm => {
            let r = chargen::validate(&c);
            if !r.errors.is_empty() {
                report(&c, &r);
                bail!("cannot confirm with errors");
            }
            let id = chargen::slug(c.name.as_deref().unwrap_or_default());
            if c.name.is_none() || !save.join("Characters").join(format!("{id}.md")).is_file() {
                bail!("write the sheet first (`chargen sheet`)");
            }
            c.confirmed = true;
            if !c.gold_granted {
                let root = save.parent().and_then(|p| p.parent());
                let book = classdata::Book::load(c.rs(), root)?;
                let gold = c.class_gold + c.background_pack(Some(&book)).gold;
                if gold > 0 {
                    let id = chargen::slug(c.name.as_deref().unwrap_or_default());
                    for l in progress::loot(
                        save,
                        progress::LootOp::Add {
                            to: &id,
                            cp: gold * 100,
                            items: &[],
                        },
                        Some(&format!("{} 起始金币", c.name.clone().unwrap_or_default())),
                    )? {
                        println!("{l}");
                    }
                }
                c.gold_granted = true;
            }
        }
    }
    chargen::store(save, &c)?;
    let r = chargen::validate(&c);
    for w in &r.errors {
        println!("error: {w}");
    }
    print_next(&c);
    Ok(())
}

fn report(c: &Creation, r: &chargen::Report) {
    let s = c.final_scores();
    println!(
        "{} | HP {} AC {} prof +{}",
        data::ABILITIES
            .iter()
            .map(|a| format!("{a} {}", s[*a]))
            .collect::<Vec<_>>()
            .join(" "),
        c.hp().map(|h| h.to_string()).unwrap_or("?".into()),
        c.ac(),
        data::prof_bonus(c.level)
    );
    for e in &r.errors {
        println!("error: {e}");
    }
    for w in &r.warnings {
        println!("warning: {w}");
    }
    println!(
        "{} error(s), {} warning(s)",
        r.errors.len(),
        r.warnings.len()
    );
}

fn npc_cmd(
    root: &Path,
    slot: Option<&str>,
    cmd: NpcCmd,
    rng: &mut dyn DiceRng,
    seeded: bool,
) -> Result<()> {
    let save = save::resolve_slot(root, slot).ok();
    match cmd {
        NpcCmd::Random {
            ruleset,
            level,
            count,
        } => {
            let rs = match (ruleset, &save) {
                (Some(r), _) => r,
                (None, Some(s)) => save::ruleset(s),
                (None, None) => "2024".into(),
            };
            check_ruleset(&rs)?;
            let level = level
                .or_else(|| {
                    save.as_ref()
                        .and_then(|s| chargen::load(s).ok())
                        .map(|c| c.level)
                })
                .unwrap_or(1);
            if !(1..=20).contains(&level) {
                bail!("level must be 1..20, got {level}");
            }
            let drafts = save.as_ref().map(|s| s.join("npc-drafts"));
            let mut start_n = 1;
            if let Some(d) = &drafts {
                fs::create_dir_all(d)?;
                while d.join(format!("{start_n}.json")).exists() {
                    start_n += 1;
                }
            }
            for i in 0..count {
                let n = npc::random(&rs, level, rng)?;
                let num = start_n + i;
                println!("draft {num}: {}", npc::summary(&n));
                if let (Some(d), Some(s)) = (&drafts, &save) {
                    let draft = serde_json::to_string_pretty(&n.creation)?;
                    atomic_write(d.join(format!("{num}.json")), draft.as_bytes())?;
                    log_rolls(s, &format!("同伴草稿 {num}"), &n.roll_texts, seeded)?;
                    rolls::append(
                        s,
                        &format!("同伴草稿 {num}"),
                        "属性生成",
                        &n.roll_texts.join("  "),
                        seeded,
                    )?;
                }
            }
            println!("show these to the player; after they confirm, add name/personality with `npc sheet <draft> --name ...`");
        }
        NpcCmd::Sheet {
            draft,
            name,
            personality,
        } => {
            let s = save.context("no save")?;
            let p = s.join("npc-drafts").join(format!("{draft}.json"));
            let mut c: Creation = serde_json::from_str(
                &fs::read_to_string(&p).with_context(|| format!("draft {draft} not found"))?,
            )?;
            c.name = Some(name.clone());
            c.personality = personality;
            c.adventure = chargen::load(&s).ok().and_then(|x| x.adventure);
            let book = classdata::Book::load(c.rs(), Some(root))?;
            let (id, md) = chargen::sheet_with(&c, "DM（同伴 NPC）", &book, Some(root))?;
            if !chargen::valid_id(&id) {
                bail!("NPC name must give an id of lowercase letters, digits and hyphens (got `{id}`)");
            }
            let md = md.replacen("kind: pc", "kind: npc", 1);
            atomic_write(s.join("NPCs").join(format!("{id}.md")), md.as_bytes())?;
            add_to_party(&s, &id)?;
            println!("wrote NPCs/{id}.md; add backstory and voice to it and NPCs/_party.md");
        }
    }
    Ok(())
}

fn data_cmd(root: &Path, cmd: DataCmd) -> Result<()> {
    match cmd {
        DataCmd::Status { ruleset } => {
            check_ruleset(&ruleset)?;
            let b = classdata::Book::load(&ruleset, Some(root))?;
            let md = classdata::md_files(&root.join("Rules").join(&ruleset));
            println!(
                "ruleset {ruleset}: {} classes (SRD + local)",
                b.classes.len()
            );
            let (reader, pages) = classdata::rulebooks(&root.join("Rules").join(&ruleset));
            if reader.is_empty() {
                println!("rules source: SRD only (no full-rulebook Markdown found)");
            } else {
                println!("rules source: FULL RULEBOOKS (local) — read these first; SRD is only a fallback");
                for f in &reader {
                    println!("  reader: {}", rel(root, f));
                }
                for f in &pages {
                    println!("  pages:  {}", rel(root, f));
                }
            }
            if classdata::import_stale(root, &ruleset) {
                println!("local import is missing or older than the books: run `talkdnd data import --ruleset {ruleset}` (talkdnd start does it automatically)");
            }
            println!("converted Markdown: {}", md.len());
            for f in &md {
                println!("  {}", rel(root, f));
            }
            println!("local data files: {}", b.local_files.len());
            for f in &b.local_files {
                println!("  {}", rel(root, f));
            }
            if !b.local_ids.is_empty() {
                println!("local entries: {}", b.local_ids.join(", "));
            }
            if md.is_empty() {
                println!("next: {}", classdata::import_hint(Some(root), &ruleset));
            }
        }
        DataCmd::Import { ruleset } => {
            check_ruleset(&ruleset)?;
            let (out, subs, bgs) = classdata::run_import(root, &ruleset)?;
            println!(
                "wrote {} ({subs} non-SRD subclass(es), {bgs} background(s))",
                rel(root, &out)
            );
        }
    }
    Ok(())
}

fn print_options(rs: &str) {
    println!("ruleset {rs} (SRD options; other books' options are allowed with a warning)");
    println!(
        "species: {}",
        data::species_list(rs)
            .iter()
            .map(|s| s.id)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "classes: {}",
        data::CLASSES
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "backgrounds: {}",
        data::background_list(rs)
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "skills: {}",
        data::SKILLS
            .iter()
            .map(|s| s.0)
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "armor: none, {}",
        data::ARMOR
            .iter()
            .map(|a| a.0)
            .collect::<Vec<_>>()
            .join(", ")
    );
}
