//! TalkDND play helper. Mechanical, non-LLM steps for the AI DM:
//! resume digest, new save skeleton, character creation, random companions,
//! checkpoints and log entries. All dice go through combat-sim's engine.

pub mod chargen;
pub mod check;
pub mod classdata;
pub mod data;
pub mod npc;
pub mod progress;
pub mod save;
pub mod start;

use anyhow::{bail, Context, Result};
use chargen::Creation;
use clap::{Parser, Subcommand};
use combat_sim::dice::{seeded_rng, step_rng, DiceRng};
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "talkdnd",
    version,
    about = "TalkDND play helper (run inside LetsPlay/)"
)]
struct Cli {
    /// LetsPlay folder (default: found from the current directory)
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

#[derive(Subcommand)]
enum Cmd {
    /// Resume: content check, background pdf2md, write and print Saves/<slot>/digest.md
    Start {
        #[arg(long)]
        no_pdf2md: bool,
    },
    /// Create a new save skeleton and start character creation
    New {
        #[arg(long)]
        ruleset: Option<String>,
        #[arg(long)]
        adventure: Option<String>,
        /// Slot date (default: today, local time)
        #[arg(long)]
        date: Option<String>,
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
    /// Ability/skill check or saving throw outside combat (modifier from the sheet; logged)
    Check {
        /// Character id or name (Characters/ or NPCs/)
        who: String,
        /// Skill id (perception), ability (wis), save:<ability> or init
        what: String,
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
    Log { text: Vec<String> },
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
        /// Milestone levelling: skip the XP check
        #[arg(long)]
        milestone: bool,
    },
    /// Coins and items per character and the shared `party` pool
    Loot {
        #[command(subcommand)]
        cmd: LootCmd,
    },
    /// List SRD options for a ruleset
    Options {
        #[arg(long, default_value = "2024")]
        ruleset: String,
    },
}

#[derive(Subcommand)]
enum XpCmd {
    /// Award XP, split evenly: `xp add 450 --to all --reason "museum guards"`
    Add {
        amount: i64,
        /// `all` (campaign party) or comma-separated names/ids
        #[arg(long, default_value = "all")]
        to: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// XP, level and next threshold per character
    Show,
}

#[derive(clap::Args)]
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

#[derive(Subcommand)]
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

#[derive(Subcommand)]
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
    /// Record a choice: ruleset|adventure|species|class|subclass|masteries|background|skills|background-skills|armor|shield|equipment|spells|name|personality|level
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

#[derive(Subcommand)]
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

#[derive(Subcommand)]
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
    match cli.cmd {
        Cmd::Start { no_pdf2md } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let d = start::digest(&root, &s, &start::Options { pdf2md: !no_pdf2md })?;
            print!("{d}");
        }
        Cmd::New {
            ruleset,
            adventure,
            date,
        } => new_save(&root, ruleset, adventure, date)?,
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
                XpCmd::Add { amount, to, reason } => {
                    if amount <= 0 {
                        bail!("XP amount must be positive");
                    }
                    progress::xp_add(&s, amount, &to, reason.as_deref())?
                }
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
        } => {
            let s = save::resolve_slot(&root, cli.save.as_deref())?;
            let r = progress::level_up(
                &s,
                &root,
                &who,
                &hp,
                class.as_deref(),
                milestone,
                rng,
                cli.seed.is_some(),
            )?;
            for l in r.lines {
                println!("{l}");
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

fn new_save(
    root: &Path,
    ruleset: Option<String>,
    adventure: Option<String>,
    date: Option<String>,
) -> Result<()> {
    if let Some(r) = &ruleset {
        check_ruleset(r)?;
    }
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
    fs::write(
        dir.join("combat/campaign.yaml"),
        format!(
            "# combat-sim campaign manifest for save {slot} (paths relative to this file)\nname: \"TalkDND {slot}\"\nlibrary: [../../../Rules/combat]\n{adv_line}characters: [../Characters, ../NPCs]\nstate_dir: .\nparty: []\nruleset: \"{rs}\"   # 2014 | 2024\n"
        ),
    )?;
    fs::write(dir.join("canon.md"), "# 即兴定论（公开）\n\n")?;
    fs::write(dir.join("dm-secrets.md"), "# DM 秘密（只给 DM）\n\n")?;
    fs::write(
        dir.join(format!("Sessions/{date}-session-00-setup.md")),
        format!(
            "# {slot} 第 0 节：建卡\n\n- [{}] 新存档，规则 {}\n",
            chrono::Local::now().format("%H:%M"),
            ruleset.as_deref().unwrap_or("待定")
        ),
    )?;
    let c = Creation {
        ruleset,
        adventure,
        level: 1,
        ..Default::default()
    };
    chargen::store(&dir, &c)?;
    println!("created Saves/{slot}");
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
    fs::write(&p, out.join("\n") + "\n")?;
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
                    if c.armor.is_none() {
                        c.armor = Some("none".into());
                    }
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
                "personality" => c.personality = Some(v),
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
            let file = save
                .join("Characters")
                .join(format!("{}.md", c.name.as_deref().unwrap_or(&id)));
            fs::write(&file, md)?;
            add_to_party(save, &id)?;
            println!(
                "wrote Characters/{} (id {id}); show it to the player, then `chargen confirm`",
                file.file_name().unwrap_or_default().to_string_lossy()
            );
            return Ok(());
        }
        ChargenCmd::Confirm => {
            let r = chargen::validate(&c);
            if !r.errors.is_empty() {
                report(&c, &r);
                bail!("cannot confirm with errors");
            }
            if c.name.is_none()
                || !save
                    .join("Characters")
                    .join(format!("{}.md", c.name.clone().unwrap_or_default()))
                    .is_file()
            {
                bail!("write the sheet first (`chargen sheet`)");
            }
            c.confirmed = true;
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
                    fs::write(
                        d.join(format!("{num}.json")),
                        serde_json::to_string_pretty(&n.creation)?,
                    )?;
                    log_rolls(s, &format!("同伴草稿 {num}"), &n.roll_texts, seeded)?;
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
            let md = md.replacen("kind: pc", "kind: npc", 1);
            fs::write(s.join("NPCs").join(format!("{name}.md")), md)?;
            add_to_party(&s, &id)?;
            println!(
                "wrote NPCs/{name}.md (id {id}); add backstory and voice to it and NPCs/_party.md"
            );
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
            let md = classdata::md_files(&root.join("Rules").join(&ruleset));
            if md.is_empty() {
                bail!("{}", classdata::import_hint(Some(root), &ruleset));
            }
            let srd = classdata::Book::srd(&ruleset);
            let df = classdata::import_markdown(&ruleset, &srd, &md)?;
            let dir = root.join(classdata::LOCAL_DIR);
            fs::create_dir_all(&dir)?;
            let out = dir.join(format!("imported-{ruleset}.yaml"));
            let head = format!("# Generated by `talkdnd data import --ruleset {ruleset}` from your own books. LOCAL ONLY: never commit.\n# Feature names by level only; check them against the PDF. Hand edits belong in another file (e.g. manual-{ruleset}.yaml), this one is overwritten.\n");
            fs::write(&out, head + &serde_norway::to_string(&df)?)?;
            let mut n = 0;
            for (cid, c) in &df.classes {
                for (sid, s) in &c.subclasses {
                    n += 1;
                    println!("  {cid}/{sid}: {} feature(s)", s.features.len());
                }
            }
            println!("wrote {} ({n} non-SRD subclass(es))", rel(root, &out));
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
