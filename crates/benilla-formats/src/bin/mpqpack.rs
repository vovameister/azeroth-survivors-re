//! Packing helper, not shipped: `mpqpack <data-dir> <list> <out.MPQ> [<seeds>]`, for
//! `scripts/trim-data.sh`.
//!
//! The pack holds every name in `<list>` (one per line, as `WOW_DATA_TRACE` writes them), every
//! ADT tile within each `<seeds>` line's reach (`<map id> <wow x> <wow y> <radius yd>`, with the
//! map's WDT and WDL and each tile's minimap art), and, closing over all of them, every texture,
//! model and WMO those files name and every variation of a sound kit one of its files belongs to:
//! a trace only holds what one run happened to stream or roll, and a tile it missed would hold the
//! loading screen up for good. Each file is read through the runtime's patch chain
//! ([`benilla_formats::Chain`]), so a patched one packs in the version that wins, into one archive
//! ([`benilla_mpq::ArchiveWriter`]). A name the chain does not hold is skipped: a trace lists the
//! probes that found nothing too.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};
use std::collections::HashMap;

use benilla_formats::{
    load_creature_catalog, load_creature_voice_catalog, load_map_catalog, load_minimap_translate,
    load_sound_kit_catalog, Chain, MapTiles, TILE_SIZE,
};
use benilla_mpq::ArchiveWriter;

/// Sound folders packed whole, whatever the traces hold: every race's combat exertions (a run plays
/// one race, the player may pick any), every weapon kind's swings and hits (the hero's weapon is
/// the look's), and the musical moments (picked by chance).
const WHOLE_SOUND_DIRS: &[&str] = &[
    "sound\\character\\playerexertions\\",
    "sound\\item\\weapons\\",
    "sound\\music\\musical moments\\",
];
/// Under `Sound\Character\` (whose emotes and speech run to hundreds of MB), only the fight's
/// files, by name: a race's wounds and death (its swings are in `PlayerExertions`).
const COMBAT_SOUND_WORDS: &[&str] = &["wound", "death"];
/// Folders whose every kit belongs to one thing, so one file heard stands for them all: a
/// creature's attack, wound and death kits, a zone's day and night music. Ambience folders are
/// flat across the world and stay out.
const KIN_SOUND_DIRS: &[&str] = &["sound\\creature\\", "sound\\music\\zonemusic\\"];
/// Tables whose named art a fight may show for any creature, packed whole by a filter on the name:
/// the blood spurts each blood level resolves to (through `SpellVisualEffectName`) and the splats
/// `UnitBlood` names (bare, without `.blp`).
const DBC_ART: &[(&str, &str)] = &[
    ("DBFilesClient\\SpellVisualEffectName.dbc", "bloodspurt"),
    ("DBFilesClient\\UnitBlood.dbc", "bloodsplat"),
];
/// Interior ambience shared by many WMOs (their `WMOAreaTable` rows), by name: the room tones.
const ROOM_TONE: (&str, &str) = ("sound\\ambience\\wmoambience", "tone");

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (data, list, out) = (
        args.next().context("data dir")?,
        args.next().context("name list")?,
        args.next().context("out .MPQ")?,
    );
    let seeds = args.next();
    let mut chain = Chain::open(Path::new(&data)).context("open patch chain")?;

    let mut names: Vec<String> = std::fs::read_to_string(&list)
        .with_context(|| format!("reading {list}"))?
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect();
    if let Some(seeds) = seeds {
        let tiles = seed_tiles(&mut chain, &seeds)?;
        println!("{seeds}: {} map files within the arenas' reach", tiles.len());
        names.extend(tiles);
    }
    for (dbc, filter) in DBC_ART {
        let bytes = chain.read(dbc)?;
        for run in strings(&bytes) {
            if !run.to_ascii_lowercase().contains(filter) {
                continue;
            }
            let file = as_file(run);
            let with_blp = format!("{file}.blp");
            if chain.contains(&file) {
                names.push(file);
            } else if chain.contains(&with_blp) {
                names.push(with_blp);
            }
        }
    }

    // Breadth-first over the references, each name once however it is spelt.
    let key = |n: &str| n.replace('/', "\\").to_ascii_lowercase();
    // A sound file to every variation of its kit (a run plays one at random), and in a
    // [`KIN_SOUND_DIRS`] folder to every kit there.
    let dir_of = |f: &str| {
        let k = key(f);
        k.rsplit_once('\\').map_or(String::new(), |(d, _)| d.to_owned())
    };
    let sounds = load_sound_kit_catalog(&mut chain)?;
    let mut by_dir: HashMap<String, Vec<String>> = HashMap::new();
    for kit in sounds.iter() {
        for (file, _) in &kit.files {
            by_dir.entry(dir_of(file)).or_default().push(file.clone());
        }
    }
    // A creature model to its whole voice (attacks, wounds, death, fidgets), through its own sound
    // set and every display's: a run hears whichever the fight rolls.
    let creatures = load_creature_catalog(&mut chain)?;
    let voices = load_creature_voice_catalog(&mut chain)?;
    let model_key = |p: &str| {
        let k = key(p);
        k.strip_suffix(".mdx").map_or(k.clone(), |s| format!("{s}.m2"))
    };
    let mut model_of: HashMap<u32, String> = HashMap::new();
    let mut voice_of: HashMap<String, Vec<String>> = HashMap::new();
    let mut add_voice = |model: &str, voice: &benilla_formats::CreatureVoice| {
        let files = voice
            .kits()
            .into_iter()
            .filter_map(|k| sounds.get(k))
            .flat_map(|kit| kit.files.iter().map(|(f, _)| f.clone()));
        voice_of.entry(model.to_owned()).or_default().extend(files);
    };
    for (id, path, _) in creatures.sized_models() {
        let model = model_key(path);
        if let Some(v) = voices.for_model(id) {
            add_voice(&model, v);
        }
        model_of.insert(id, model);
    }
    for (display, model) in creatures.display_models() {
        if let (Some(m), Some(v)) = (model_of.get(&model), voices.for_display(display)) {
            add_voice(m, v);
        }
    }
    let mut kin: HashMap<String, Vec<String>> = HashMap::new();
    for kit in sounds.iter() {
        for (file, _) in &kit.files {
            let all = kit.files.iter().map(|(f, _)| f.clone());
            kin.entry(key(file)).or_default().extend(all);
        }
    }
    for (dir, files) in &by_dir {
        if KIN_SOUND_DIRS.iter().any(|p| dir.starts_with(p)) {
            for f in files {
                kin.entry(key(f)).or_default().extend(files.iter().cloned());
            }
        }
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for n in names {
        if seen.insert(key(&n)) {
            queue.push_back(n);
        }
    }
    for (dir, files) in &by_dir {
        let whole = WHOLE_SOUND_DIRS.iter().any(|p| dir.starts_with(p));
        let combat = |f: &&String| {
            let name = key(f);
            let file = name.rsplit_once('\\').map_or(name.as_str(), |(_, n)| n);
            COMBAT_SOUND_WORDS.iter().any(|w| file.contains(w))
        };
        let tone = |f: &&String| dir.starts_with(ROOM_TONE.0) && key(f).contains(ROOM_TONE.1);
        if whole || dir.starts_with("sound\\character\\") || dir.starts_with(ROOM_TONE.0) {
            for f in files.iter().filter(|f| whole || combat(f) || tone(f)) {
                if seen.insert(key(f)) {
                    queue.push_back(f.clone());
                }
            }
        }
    }
    // The archives already beside `out` are kept whole under it; every name of theirs that the
    // full chain takes from a dropped archive (a patch's override or tombstone) goes in too, or
    // the trimmed set would quietly serve the stale lower copy.
    let deleted = shadowed(&chain, Path::new(&out), &mut seen, &mut queue)?;
    let mut writer = ArchiveWriter::create(&out).with_context(|| format!("creating {out}"))?;
    for name in &deleted {
        writer.delete(name)?;
    }
    let (mut files, mut bytes, mut missing, mut referenced) = (0usize, 0u64, 0usize, 0usize);
    while let Some(name) = queue.pop_front() {
        if !chain.contains(&name) {
            missing += 1;
            continue;
        }
        let data = chain.read(&name)?;
        let kits = kin
            .get(&key(&name))
            .into_iter()
            .chain(voice_of.get(&key(&name)))
            .flatten()
            .cloned();
        for r in references(&chain, &name, &data).into_iter().chain(kits) {
            if seen.insert(key(&r)) {
                referenced += 1;
                queue.push_back(r);
            }
        }
        writer.add(&name, &data)?;
        files += 1;
        bytes += data.len() as u64;
    }
    writer.finish()?;
    let packed = std::fs::metadata(&out)?.len();
    println!(
        "{out}: {files} files ({referenced} by reference), {} MB as read, {} MB packed \
         ({missing} names not in the chain)",
        bytes >> 20,
        packed >> 20
    );
    Ok(())
}

/// The names of the `.MPQ`s beside `out` (the archives kept whole) that the full chain resolves to
/// a dropped archive: an override is queued for the pack (read as the chain resolves it), and a
/// tombstone is returned, to be packed as one.
fn shadowed(
    chain: &Chain,
    out: &Path,
    seen: &mut HashSet<String>,
    queue: &mut VecDeque<String>,
) -> Result<Vec<String>> {
    let dir = out.parent().unwrap_or(Path::new("."));
    let lower = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    let mut kept = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let path = entry?.path();
        if lower(&path).ends_with(".mpq") && lower(&path) != lower(out) {
            kept.push(path);
        }
    }
    let kept_names: HashSet<String> = kept.iter().map(|p| lower(p)).collect();
    let (mut overrides, mut deleted, mut tombstoned) = (0usize, Vec::new(), HashSet::new());
    for path in &kept {
        let archive = benilla_mpq::Archive::open(path)
            .with_context(|| format!("opening {}", path.display()))?;
        let Ok(listfile) = archive.read_file("(listfile)") else {
            continue;
        };
        for name in String::from_utf8_lossy(&listfile).split([';', '\r', '\n']) {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let Some(winner) = chain.find_file_archive(name) else {
                continue;
            };
            if kept_names.contains(&lower(winner)) {
                continue;
            }
            // A tombstone even when a trace queued the name: the pack would skip it as missing.
            let key = name.replace('/', "\\").to_ascii_lowercase();
            if !chain.contains(name) {
                if tombstoned.insert(key) {
                    deleted.push(name.to_owned());
                }
            } else if seen.insert(key) {
                overrides += 1;
                queue.push_back(name.to_owned());
            }
        }
    }
    println!(
        "{} kept archives: {overrides} names overridden and {} deleted by a dropped one",
        kept.len(),
        deleted.len()
    );
    Ok(deleted)
}

/// The ADT tiles (and each map's WDT and WDL, and each tile's minimap art) every `<seeds>` line
/// reaches.
fn seed_tiles(chain: &mut Chain, path: &str) -> Result<Vec<String>> {
    let catalog = load_map_catalog(chain)?;
    let minimap = load_minimap_translate(chain)?;
    let mut out = Vec::new();
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {path}"))?;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [id, x, y, r] = f[..] else {
            anyhow::bail!("{path}: want `<map id> <x> <y> <radius yd>`, got {line:?}");
        };
        let id: u32 = id.parse()?;
        let (x, y, r): (f32, f32, f32) = (x.parse()?, y.parse()?, r.parse()?);
        let dir = catalog
            .directory(id)
            .with_context(|| format!("map {id} not in Map.dbc"))?
            .to_owned();
        let tiles = MapTiles::load(chain, &dir)?;
        out.push(format!("World\\Maps\\{dir}\\{dir}.wdt"));
        out.push(format!("World\\Maps\\{dir}\\{dir}.wdl"));
        // The tiles the square `r` either side of the centre touches: the streamer's window is a
        // square round the viewer, and the viewer stays within the leash.
        let (ax, ay) = tiles.tile_at(x - r, y - r);
        let (bx, by) = tiles.tile_at(x + r, y + r);
        let (xs, ys) = (ax.min(bx)..=ax.max(bx), ay.min(by)..=ay.max(by));
        let radius = (r / TILE_SIZE).ceil() as u32 + 1;
        let reach = tiles.existing_in_radius(x, y, radius);
        for (tx, ty) in reach.into_iter().filter(|(tx, ty)| xs.contains(tx) && ys.contains(ty)) {
            out.push(format!("World\\Maps\\{dir}\\{dir}_{tx}_{ty}.adt"));
            if let Some(art) = minimap.tile(&dir, tx, ty) {
                out.push(format!("textures\\Minimap\\{art}"));
            }
        }
    }
    Ok(out)
}

/// The files a terrain tile, WMO or model names, as the chain holds them: textures, models
/// (`.mdx`/`.mdl` are the `.m2` the client loads) and WMOs, and a WMO root's groups.
fn references(chain: &Chain, name: &str, data: &[u8]) -> Vec<String> {
    let lower = name.to_ascii_lowercase();
    if !(lower.ends_with(".adt")
        || lower.ends_with(".wmo")
        || lower.ends_with(".m2")
        || lower.ends_with(".mdx"))
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    for s in strings(data) {
        let path = as_file(s);
        let l = path.to_ascii_lowercase();
        if !(l.ends_with(".blp") || l.ends_with(".m2") || l.ends_with(".wmo")) {
            continue;
        }
        // A chunk tag or length byte can run into the path: try it whole, then a few bytes in.
        if let Some(found) = (0..path.len().min(8))
            .map(|i| &path[i..])
            .find(|p| chain.contains(p))
        {
            out.push(found.to_owned());
        }
    }
    // A WMO root (not a `_NNN` group) loads its groups by number.
    if let Some(stem) = lower.strip_suffix(".wmo") {
        let is_group = stem.len() > 4
            && stem.as_bytes()[stem.len() - 4] == b'_'
            && stem[stem.len() - 3..].bytes().all(|b| b.is_ascii_digit());
        if !is_group {
            let stem = &name[..name.len() - 4];
            for i in 0.. {
                let group = format!("{stem}_{i:03}.wmo");
                if !chain.contains(&group) {
                    break;
                }
                out.push(group);
            }
        }
    }
    out
}

/// The printable ASCII runs in `data` at least 5 bytes long: where a file's paths are.
fn strings(data: &[u8]) -> impl Iterator<Item = &str> {
    data.split(|b| !(0x20..0x7f).contains(b))
        .filter(|r| r.len() >= 5)
        .filter_map(|r| std::str::from_utf8(r).ok())
}

/// A named model as the client loads it: `.mdx`/`.mdl` are the `.m2` on disk.
fn as_file(name: &str) -> String {
    let l = name.to_ascii_lowercase();
    if l.ends_with(".mdx") || l.ends_with(".mdl") {
        format!("{}.m2", &name[..name.len() - 4])
    } else {
        name.to_owned()
    }
}
