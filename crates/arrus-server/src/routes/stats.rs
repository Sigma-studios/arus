//! The admin's statistics: the whole site, and one account in detail.

use std::collections::HashMap;

use arrus_api::{
    AccountDetailStats, AccountStats, AccountTeamStats, DayStats, STATS_MAX_DAYS, SiteStats, StatChoreography,
    StatTotals, TeamRole, TeamStats,
};
use axum::Json;
use axum::extract::{Path, Query, State};
use rusqlite::{Connection, params};
use serde::Deserialize;

use crate::activity::{Kind, day_of};
use crate::auth::Auth;
use crate::error::{Error, Result};
use crate::{AppState, db};

#[derive(Deserialize)]
pub struct DaysQuery {
    days: Option<u32>,
}

fn window(q: &DaysQuery) -> u32 {
    q.days.unwrap_or(30).clamp(1, STATS_MAX_DAYS)
}

pub async fn site(State(state): State<AppState>, auth: Auth, Query(q): Query<DaysQuery>) -> Result<Json<SiteStats>> {
    auth.user.require_admin()?;
    let days = window(&q);
    state
        .db(move |s, conn| {
            s.activity.flush(conn)?;
            Ok(Json(site_stats(conn, day_of(db::now()), days)?))
        })
        .await
}

pub async fn account(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
    Query(q): Query<DaysQuery>,
) -> Result<Json<AccountDetailStats>> {
    auth.user.require_admin()?;
    let days = window(&q);
    state
        .db(move |s, conn| {
            s.activity.flush(conn)?;
            Ok(Json(account_stats(conn, id, day_of(db::now()), days)?))
        })
        .await
}

/// Empty days from `today - days + 1` to `today`.
fn empty_days(today: i64, days: u32) -> Vec<DayStats> {
    let from = today - days as i64 + 1;
    (from..=today).map(|day| DayStats { day, ..Default::default() }).collect()
}

fn slot(daily: &mut [DayStats], day: i64) -> Option<&mut DayStats> {
    let from = daily.first()?.day;
    daily.get_mut(usize::try_from(day - from).ok()?)
}

/// Add the activity rows (`account` `None`: everyone) to `daily`.
fn fill_activity(conn: &Connection, daily: &mut [DayStats], account: Option<i64>) -> Result<()> {
    let Some(from) = daily.first().map(|d| d.day) else { return Ok(()) };
    let mut stmt = conn.prepare(
        "SELECT day, kind, SUM(count) FROM activity WHERE day >= ?1 AND (?2 IS NULL OR account_id = ?2) \
         GROUP BY day, kind",
    )?;
    let rows = stmt.query_map(params![from, account], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
    for row in rows {
        let (day, kind, n) = row?;
        let (Some(d), Some(kind)) = (slot(daily, day), Kind::parse(&kind)) else { continue };
        let n = n.max(0) as u64;
        match kind {
            Kind::Minutes => d.minutes += n,
            Kind::Logins => d.logins += n,
            Kind::Edits => d.edits += n,
            Kind::Created => d.created += n,
            Kind::Exports => d.exports += n,
            Kind::Uploads => d.uploads += n,
        }
    }
    let mut stmt = conn.prepare(
        "SELECT day, COUNT(DISTINCT account_id) FROM activity WHERE day >= ?1 AND (?2 IS NULL OR account_id = ?2) \
         GROUP BY day",
    )?;
    for row in stmt.query_map(params![from, account], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (day, n) = row?;
        if let Some(d) = slot(daily, day) {
            d.active = n as u32;
        }
    }
    Ok(())
}

fn count(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<u32> {
    Ok(conn.query_row(sql, p, |r| r.get::<_, i64>(0))? as u32)
}

pub fn site_stats(conn: &Connection, today: i64, days: u32) -> Result<SiteStats> {
    let mut daily = empty_days(today, days);
    let from_secs = daily[0].day * 86_400;
    {
        let mut stmt = conn.prepare("SELECT created_at FROM accounts WHERE created_at >= ?1")?;
        for t in stmt.query_map([from_secs], |r| r.get::<_, i64>(0))? {
            if let Some(d) = slot(&mut daily, day_of(t?)) {
                d.signups += 1;
            }
        }
    }
    fill_activity(conn, &mut daily, None)?;

    let active_since = |n: i64| count(conn, "SELECT COUNT(DISTINCT account_id) FROM activity WHERE day > ?1", [today - n]);
    let new_since = |n: i64| count(conn, "SELECT COUNT(*) FROM accounts WHERE created_at >= ?1", [(today - n + 1) * 86_400]);
    let totals = StatTotals {
        accounts: count(conn, "SELECT COUNT(*) FROM accounts", [])?,
        premium: count(conn, "SELECT COUNT(*) FROM accounts WHERE is_premium", [])?,
        admins: count(conn, "SELECT COUNT(*) FROM accounts WHERE is_admin", [])?,
        teams: count(conn, "SELECT COUNT(*) FROM teams", [])?,
        choreographies: count(conn, "SELECT COUNT(*) FROM choreographies", [])?,
        music_tracks: count(conn, "SELECT COUNT(*) FROM choreographies WHERE music_file IS NOT NULL", [])?,
        bytes_used: crate::storage::usage(conn)?,
        active_1: active_since(1)?,
        active_7: active_since(7)?,
        active_30: active_since(30)?,
        new_7: new_since(7)?,
        new_30: new_since(30)?,
        tracking_since: conn.query_row("SELECT MIN(day) FROM activity", [], |r| r.get(0))?,
    };
    Ok(SiteStats {
        today,
        days,
        totals,
        daily,
        accounts: load_account_stats(conn, None, today - days as i64 + 1)?,
        teams: load_teams(conn, None)?.into_iter().map(|(t, _)| t).collect(),
    })
}

fn load_account_stats(conn: &Connection, only: Option<i64>, from_day: i64) -> Result<Vec<AccountStats>> {
    // Small numbers of accounts: one row each, with subqueries.
    let sum = |kind: &str, window: bool| {
        format!(
            "(SELECT COALESCE(SUM(count), 0) FROM activity WHERE account_id = a.id AND kind = '{kind}'{})",
            if window { " AND day >= ?2" } else { "" }
        )
    };
    let sql = format!(
        "SELECT a.id, a.nickname, a.email, a.is_admin, a.is_premium, a.created_at, \
         (SELECT MAX(day) FROM activity WHERE account_id = a.id), \
         (SELECT COUNT(DISTINCT day) FROM activity WHERE account_id = a.id AND day >= ?2), \
         {}, {}, {}, {}, {}, {}, \
         (SELECT COUNT(*) FROM team_members WHERE account_id = a.id), \
         (SELECT COUNT(*) FROM teams WHERE owner_id = a.id), \
         (SELECT COUNT(*) FROM choreographies c JOIN teams t ON t.id = c.team_id WHERE t.owner_id = a.id), \
         (SELECT COALESCE(SUM(c.size_bytes + COALESCE(c.music_bytes, 0)), 0) FROM choreographies c \
          JOIN teams t ON t.id = c.team_id WHERE t.owner_id = a.id) \
         FROM accounts a WHERE ?1 IS NULL OR a.id = ?1 ORDER BY a.nickname_lower",
        sum("minutes", true),
        sum("minutes", false),
        sum("edits", true),
        sum("edits", false),
        sum("logins", false),
        sum("exports", false),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![only, from_day], |r| {
        Ok(AccountStats {
            id: r.get(0)?,
            nickname: r.get(1)?,
            email: r.get(2)?,
            is_admin: r.get(3)?,
            is_premium: r.get(4)?,
            created_at: r.get(5)?,
            last_active_day: r.get(6)?,
            active_days: r.get::<_, i64>(7)? as u32,
            minutes: r.get::<_, i64>(8)? as u64,
            minutes_total: r.get::<_, i64>(9)? as u64,
            edits: r.get::<_, i64>(10)? as u64,
            edits_total: r.get::<_, i64>(11)? as u64,
            logins_total: r.get::<_, i64>(12)? as u64,
            exports_total: r.get::<_, i64>(13)? as u64,
            teams: r.get::<_, i64>(14)? as u32,
            owned_teams: r.get::<_, i64>(15)? as u32,
            choreographies: r.get::<_, i64>(16)? as u32,
            bytes: r.get::<_, i64>(17)? as u64,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Every team (or those `account` is in, with its role), largest first.
fn load_teams(conn: &Connection, account: Option<i64>) -> Result<Vec<(TeamStats, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, o.nickname, \
         (SELECT COUNT(*) FROM team_members m WHERE m.team_id = t.id), \
         (SELECT COUNT(*) FROM choreographies c WHERE c.team_id = t.id), \
         (SELECT COALESCE(SUM(c.size_bytes + COALESCE(c.music_bytes, 0)), 0) FROM choreographies c WHERE c.team_id = t.id) AS bytes, \
         (SELECT role FROM team_members m WHERE m.team_id = t.id AND m.account_id = ?1) AS role \
         FROM teams t LEFT JOIN accounts o ON o.id = t.owner_id \
         WHERE ?1 IS NULL OR role IS NOT NULL ORDER BY bytes DESC, t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([account], |r| {
        Ok((
            TeamStats {
                id: r.get(0)?,
                name: r.get(1)?,
                owner: r.get(2)?,
                members: r.get::<_, i64>(3)? as u32,
                choreographies: r.get::<_, i64>(4)? as u32,
                bytes: r.get::<_, i64>(5)? as u64,
            },
            r.get(6)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn account_stats(conn: &Connection, id: i64, today: i64, days: u32) -> Result<AccountDetailStats> {
    let from_day = today - days as i64 + 1;
    let account = load_account_stats(conn, Some(id), from_day)?.into_iter().next().ok_or_else(Error::not_found)?;
    let mut daily = empty_days(today, days);
    if let Some(d) = slot(&mut daily, day_of(account.created_at)) {
        d.signups = 1;
    }
    fill_activity(conn, &mut daily, Some(id))?;
    let teams: Vec<AccountTeamStats> = load_teams(conn, Some(id))?
        .into_iter()
        .map(|(team, role)| AccountTeamStats {
            team,
            role: role.as_deref().and_then(TeamRole::parse).unwrap_or(TeamRole::Guest),
        })
        .collect();
    let names: HashMap<i64, String> = teams.iter().map(|t| (t.team.id, t.team.name.clone())).collect();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.team_id, c.name, c.formation_count, c.dancer_count, c.size_bytes + COALESCE(c.music_bytes, 0), \
         c.music_file IS NOT NULL, c.created_at, c.updated_at, c.updated_by \
         FROM choreographies c JOIN team_members m ON m.team_id = c.team_id AND m.account_id = ?1 \
         ORDER BY c.updated_at DESC",
    )?;
    let choreographies = stmt
        .query_map([id], |r| {
            let team: i64 = r.get(1)?;
            Ok(StatChoreography {
                id: r.get(0)?,
                team: names.get(&team).cloned().unwrap_or_default(),
                name: r.get(2)?,
                formation_count: r.get::<_, i64>(3)? as u32,
                dancer_count: r.get::<_, i64>(4)? as u32,
                bytes: r.get::<_, i64>(5)? as u64,
                has_music: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
                updated_by: r.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(AccountDetailStats { today, days, account, daily, teams, choreographies })
}
