//! Team invitations from the in-game chat: a team leader (marked in the panel) invites a
//! connected player, the player accepts, and the server puts them in the team without the
//! admin having to open the panel. Everything is a normal chat line, so players need no
//! admin key. People who join or leave this way are kept in `m182_members.json`, which the
//! panel never overwrites.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::groups::{self, Member, MembersFile, TeamDef};
use super::roster::same_name;

pub const PREFIXES: [&str; 4] = ["!equipo", "/equipo", "!team", "/team"];
const INVITE_LIFE: Duration = Duration::from_secs(120);
const MAX_ENTRIES: usize = 400;
const MAX_PENDING: usize = 64;

/// A connected player as the chat hook sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Person {
    pub entity: u32,
    pub name: String,
    pub id: Option<String>,
}

struct Invite {
    team: String,
    inviter: String,
    at: Instant,
}

#[derive(Default)]
pub struct Pending {
    by_entity: HashMap<u32, Invite>,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

pub fn is_command(text: &str) -> bool {
    let t = text.trim_start();
    PREFIXES.iter().any(|p| {
        t.len() >= p.len() && t.is_char_boundary(p.len()) && t[..p.len()].eq_ignore_ascii_case(p) && t[p.len()..].chars().next().is_none_or(char::is_whitespace)
    })
}

/// Answers a team command typed by `who`. The returned line is what the chat shows.
pub fn handle(text: &str, who: &Person, online: &[Person], dir: &Path) -> Option<String> {
    let Ok(mut guard) = PENDING.lock() else { return None };
    let pending = guard.get_or_insert_with(Pending::default);
    handle_with(pending, Instant::now(), text, who, online, dir)
}

struct Words {
    spanish: bool,
}

impl Words {
    fn pick<'a>(&self, es: &'a str, en: &'a str) -> &'a str {
        if self.spanish { es } else { en }
    }
}

fn next_token(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

fn load_teams(dir: &Path) -> Vec<TeamDef> {
    let mut teams = std::fs::read_to_string(dir.join(groups::TEAMS_FILE)).map(|t| groups::parse_team_defs(&t)).unwrap_or_default();
    if let Ok(text) = std::fs::read_to_string(dir.join(groups::MEMBERS_FILE)) {
        groups::apply_members(&mut teams, &text);
    }
    teams
}

fn team_of(teams: &[TeamDef], p: &Person) -> Option<usize> {
    match groups::team_for_player(teams, u32::MAX, Some(&p.name), p.entity, p.id.as_deref()) {
        0 => None,
        n => Some(n as usize - 1),
    }
}

fn is_leader(team: &TeamDef, p: &Person) -> bool {
    let id = p.id.as_deref().map(|i| i.trim().to_lowercase());
    team.leaders.iter().any(|l| id.as_deref() == Some(l.as_str()) || l.strip_prefix("name:").is_some_and(|n| same_name(n, &p.name)) || same_name(l, &p.name))
}

/// A player by exact name, or by the only name that starts with what was typed.
fn find_online<'a>(online: &'a [Person], typed: &str) -> Result<&'a Person, bool> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err(false);
    }
    if let Some(p) = online.iter().find(|p| same_name(&p.name, typed)) {
        return Ok(p);
    }
    let lower = typed.to_lowercase();
    let starts: Vec<&Person> = online.iter().filter(|p| p.name.to_lowercase().starts_with(&lower)).collect();
    match starts.as_slice() {
        [one] => Ok(one),
        [] => Err(false),
        _ => Err(true),
    }
}

fn read_members(dir: &Path) -> MembersFile {
    std::fs::read_to_string(dir.join(groups::MEMBERS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_members(dir: &Path, file: &MembersFile) -> Result<(), String> {
    let row = |m: &Member| serde_json::json!({"team": m.team, "id": m.id, "name": m.name});
    let body = serde_json::json!({
        "joined": file.joined.iter().map(row).collect::<Vec<_>>(),
        "left": file.left.iter().map(row).collect::<Vec<_>>(),
    });
    let tmp = dir.join("m182_members.json.tmp");
    std::fs::write(&tmp, body.to_string()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(groups::MEMBERS_FILE)).map_err(|e| e.to_string())
}

fn same_person(m: &Member, id: &str, name: &str) -> bool {
    (!id.is_empty() && m.id.trim().eq_ignore_ascii_case(id)) || (id.is_empty() && same_name(&m.name, name))
}

fn put_in(dir: &Path, team: &str, p: &Person) -> Result<(), String> {
    let mut file = read_members(dir);
    let id = p.id.clone().unwrap_or_default();
    file.joined.retain(|m| !same_person(m, &id, &p.name));
    file.left.retain(|m| !(same_name(&m.team, team) && same_person(m, &id, &p.name)));
    file.joined.push(Member { team: team.to_string(), id, name: p.name.clone() });
    if file.joined.len() > MAX_ENTRIES {
        file.joined.remove(0);
    }
    write_members(dir, &file)
}

fn take_out(dir: &Path, team: &str, p: &Person) -> Result<(), String> {
    let mut file = read_members(dir);
    let id = p.id.clone().unwrap_or_default();
    file.joined.retain(|m| !same_person(m, &id, &p.name));
    file.left.retain(|m| !(same_name(&m.team, team) && same_person(m, &id, &p.name)));
    file.left.push(Member { team: team.to_string(), id, name: p.name.clone() });
    if file.left.len() > MAX_ENTRIES {
        file.left.remove(0);
    }
    write_members(dir, &file)
}

pub fn handle_with(pending: &mut Pending, now: Instant, text: &str, who: &Person, online: &[Person], dir: &Path) -> Option<String> {
    let (head, rest) = next_token(text.trim_start());
    let w = Words { spanish: head.to_ascii_lowercase().ends_with("equipo") };
    let (sub, rest) = next_token(rest);
    let sub = sub.to_ascii_lowercase();
    pending.by_entity.retain(|_, i| now.duration_since(i.at) < INVITE_LIFE);
    let teams = load_teams(dir);
    let mine = team_of(&teams, who);
    let prefix = |team: &str| format!("[{} {}]", w.pick("Equipo", "Team"), team);
    let failed = |e: String| format!("[M182] {}: {e}", w.pick("no se pudo guardar", "could not save"));

    match sub.as_str() {
        "invitar" | "invite" => {
            let Some(index) = mine.filter(|i| is_leader(&teams[*i], who)) else {
                return Some(w.pick("[M182] Solo el líder de un equipo puede invitar.", "[M182] Only a team leader can invite.").to_string());
            };
            let team = &teams[index];
            let target = match find_online(online, rest) {
                Ok(t) => t,
                Err(true) => return Some(w.pick("[M182] Hay varios jugadores con ese nombre; escríbelo completo.", "[M182] Several players match; type the full name.").to_string()),
                Err(false) => return Some(w.pick("[M182] No encuentro a ese jugador conectado. Uso: !equipo invitar <nombre>", "[M182] I cannot find that player online. Use: !team invite <name>").to_string()),
            };
            if target.entity == who.entity {
                return Some(w.pick("[M182] No puedes invitarte a ti mismo.", "[M182] You cannot invite yourself.").to_string());
            }
            if team_of(&teams, target).is_some() {
                return Some(format!("[M182] {} {}", target.name, w.pick("ya está en un equipo; debe salir primero (!equipo salir).", "is already in a team; they must leave first (!team leave).")));
            }
            if pending.by_entity.len() >= MAX_PENDING {
                return Some(w.pick("[M182] Hay demasiadas invitaciones pendientes; espera un momento.", "[M182] Too many pending invitations; wait a moment.").to_string());
            }
            pending.by_entity.insert(target.entity, Invite { team: team.name.clone(), inviter: who.name.clone(), at: now });
            Some(format!(
                "{} {} {} {}. {}, {}",
                prefix(&team.name),
                who.name,
                w.pick("invitó a", "invited"),
                target.name,
                target.name,
                w.pick("escribe !equipo aceptar (vale 2 minutos) o !equipo rechazar.", "type !team accept (valid for 2 minutes) or !team decline.")
            ))
        }
        "aceptar" | "accept" => {
            let Some(invite) = pending.by_entity.get(&who.entity) else {
                return Some(w.pick("[M182] No tienes ninguna invitación pendiente.", "[M182] You have no pending invitation.").to_string());
            };
            if team_of(&teams, who).is_some() {
                pending.by_entity.remove(&who.entity);
                return Some(w.pick("[M182] Ya estás en un equipo; sal primero con !equipo salir.", "[M182] You are already in a team; leave it first with !team leave.").to_string());
            }
            let (team, inviter) = (invite.team.clone(), invite.inviter.clone());
            if !teams.iter().any(|t| same_name(&t.name, &team)) {
                pending.by_entity.remove(&who.entity);
                return Some(w.pick("[M182] Ese equipo ya no existe.", "[M182] That team no longer exists.").to_string());
            }
            pending.by_entity.remove(&who.entity);
            match put_in(dir, &team, who) {
                Ok(()) => Some(format!("{} {} {} ({}).", prefix(&team), who.name, w.pick("se unió al equipo", "joined the team"), inviter)),
                Err(e) => Some(failed(e)),
            }
        }
        "rechazar" | "decline" => {
            if pending.by_entity.remove(&who.entity).is_some() {
                Some(format!("[M182] {} {}.", who.name, w.pick("rechazó la invitación", "declined the invitation")))
            } else {
                Some(w.pick("[M182] No tienes ninguna invitación pendiente.", "[M182] You have no pending invitation.").to_string())
            }
        }
        "salir" | "leave" => {
            let Some(index) = mine else {
                return Some(w.pick("[M182] No estás en ningún equipo.", "[M182] You are not in a team.").to_string());
            };
            let team = teams[index].name.clone();
            match take_out(dir, &team, who) {
                Ok(()) => Some(format!("{} {} {}.", prefix(&team), who.name, w.pick("salió del equipo", "left the team"))),
                Err(e) => Some(failed(e)),
            }
        }
        "expulsar" | "kick" => {
            let Some(index) = mine.filter(|i| is_leader(&teams[*i], who)) else {
                return Some(w.pick("[M182] Solo el líder de un equipo puede expulsar.", "[M182] Only a team leader can remove people.").to_string());
            };
            let team = &teams[index];
            let target = match find_online(online, rest) {
                Ok(t) => t,
                Err(true) => return Some(w.pick("[M182] Hay varios jugadores con ese nombre; escríbelo completo.", "[M182] Several players match; type the full name.").to_string()),
                Err(false) => return Some(w.pick("[M182] No encuentro a ese jugador conectado. Uso: !equipo expulsar <nombre>", "[M182] I cannot find that player online. Use: !team kick <name>").to_string()),
            };
            if target.entity == who.entity {
                return Some(w.pick("[M182] Para salir tú, usa !equipo salir.", "[M182] To leave yourself, use !team leave.").to_string());
            }
            if team_of(&teams, target) != Some(index) {
                return Some(format!("[M182] {} {}", target.name, w.pick("no está en tu equipo.", "is not in your team.")));
            }
            if is_leader(team, target) {
                return Some(w.pick("[M182] A un líder solo lo quita el administrador.", "[M182] Only the admin can remove a leader.").to_string());
            }
            match take_out(dir, &team.name, target) {
                Ok(()) => Some(format!("{} {} {}.", prefix(&team.name), target.name, w.pick("fue expulsado del equipo", "was removed from the team"))),
                Err(e) => Some(failed(e)),
            }
        }
        _ => Some(
            w.pick(
                "[M182] Equipos: !equipo invitar <nombre> (líder) · aceptar · rechazar · salir · expulsar <nombre> (líder)",
                "[M182] Teams: !team invite <name> (leader) · accept · decline · leave · kick <name> (leader)",
            )
            .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("m182_invites_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join(groups::TEAMS_FILE),
            r#"{"teams":[{"name":"Rojo","leaders":["steamid:1"],"people":[{"key":"steamid:1","name":"Ana"},{"key":"steamid:2","name":"Luis"}]},{"name":"Azul","people":[]}]}"#,
        )
        .unwrap();
        d
    }

    fn person(entity: u32, name: &str, id: Option<&str>) -> Person {
        Person { entity, name: name.into(), id: id.map(Into::into) }
    }

    #[test]
    fn recognises_commands() {
        assert!(is_command("!equipo invitar Pedro"));
        assert!(is_command("  /TEAM accept"));
        assert!(is_command("!equipo"));
        assert!(!is_command("!equipos"));
        assert!(!is_command("hola equipo"));
    }

    #[test]
    fn leader_invites_and_player_joins_then_leaves() {
        let d = dir("flow");
        let (ana, luis, pedro) = (person(1, "Ana", Some("steamid:1")), person(2, "Luis", Some("steamid:2")), person(3, "Pedro", Some("steamid:9")));
        let online = vec![ana.clone(), luis.clone(), pedro.clone()];
        let mut p = Pending::default();
        let t = Instant::now();

        let line = handle_with(&mut p, t, "!equipo invitar ped", &ana, &online, &d).unwrap();
        assert!(line.contains("Ana invitó a Pedro"), "{line}");
        // a member who is not leader cannot invite
        assert!(handle_with(&mut p, t, "!equipo invitar Pedro", &luis, &online, &d).unwrap().contains("Solo el líder"));
        // someone without invitation cannot accept
        assert!(handle_with(&mut p, t, "!equipo aceptar", &luis, &online, &d).unwrap().contains("ninguna invitación"));
        let line = handle_with(&mut p, t, "!equipo aceptar", &pedro, &online, &d).unwrap();
        assert!(line.contains("Pedro se unió al equipo"), "{line}");
        let teams = load_teams(&d);
        assert_eq!(groups::team_for_player(&teams, 99, Some("Pedro"), 3, Some("steamid:9")), 1);

        // already in a team: cannot be invited again
        assert!(handle_with(&mut p, t, "!team invite Pedro", &ana, &online, &d).unwrap().contains("already in a team"));
        // the leader removes him, a leader cannot be removed
        assert!(handle_with(&mut p, t, "!equipo expulsar Ana", &ana, &online, &d).unwrap().contains("salir"));
        assert!(handle_with(&mut p, t, "!equipo expulsar Pedro", &ana, &online, &d).unwrap().contains("fue expulsado"));
        let teams = load_teams(&d);
        assert_eq!(groups::team_for_player(&teams, 99, Some("Pedro"), 3, Some("steamid:9")), 0);
        // an admin-defined member can leave by chat as well
        assert!(handle_with(&mut p, t, "!equipo salir", &luis, &online, &d).unwrap().contains("salió del equipo"));
        let teams = load_teams(&d);
        assert_eq!(groups::team_for_player(&teams, 99, Some("Luis"), 2, Some("steamid:2")), 0);
        assert_eq!(groups::team_for_player(&teams, 99, Some("Ana"), 1, Some("steamid:1")), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn invitations_expire_and_decline_works() {
        let d = dir("expire");
        let (ana, pedro) = (person(1, "Ana", Some("steamid:1")), person(3, "Pedro", Some("steamid:9")));
        let online = vec![ana.clone(), pedro.clone()];
        let mut p = Pending::default();
        let t = Instant::now();
        handle_with(&mut p, t, "!equipo invitar Pedro", &ana, &online, &d).unwrap();
        let late = t + INVITE_LIFE + Duration::from_secs(1);
        assert!(handle_with(&mut p, late, "!equipo aceptar", &pedro, &online, &d).unwrap().contains("ninguna invitación"));
        handle_with(&mut p, late, "!equipo invitar Pedro", &ana, &online, &d).unwrap();
        assert!(handle_with(&mut p, late, "!equipo rechazar", &pedro, &online, &d).unwrap().contains("rechazó"));
        assert!(!d.join(groups::MEMBERS_FILE).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ambiguous_and_unknown_names_are_reported() {
        let d = dir("names");
        let ana = person(1, "Ana", Some("steamid:1"));
        let online = vec![ana.clone(), person(3, "Pedro", None), person(4, "Pedra", None)];
        let mut p = Pending::default();
        let t = Instant::now();
        assert!(handle_with(&mut p, t, "!equipo invitar Ped", &ana, &online, &d).unwrap().contains("varios"));
        assert!(handle_with(&mut p, t, "!equipo invitar Zoe", &ana, &online, &d).unwrap().contains("No encuentro"));
        assert!(handle_with(&mut p, t, "!equipo invitar Ana", &ana, &online, &d).unwrap().contains("ti mismo"));
        assert!(handle_with(&mut p, t, "!equipo", &ana, &online, &d).unwrap().contains("Equipos:"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
