use arrus_api::{Storage, TeamUsage};
use axum::Json;
use axum::extract::State;

use crate::auth::Auth;
use crate::error::Result;
use crate::{AppState, storage};

pub async fn storage(State(state): State<AppState>, auth: Auth) -> Result<Json<Storage>> {
    let is_admin = auth.user.is_admin();
    state
        .db(move |s, conn| {
            let used = storage::usage(conn)?;
            let mut teams = Vec::new();
            if is_admin {
                let mut stmt = conn.prepare(
                    "SELECT t.id, t.name, COALESCE(SUM(c.size_bytes + COALESCE(c.music_bytes, 0)), 0) \
                     FROM teams t LEFT JOIN choreographies c ON c.team_id = t.id \
                     GROUP BY t.id ORDER BY t.name COLLATE NOCASE",
                )?;
                teams = stmt
                    .query_map([], |r| {
                        Ok(TeamUsage { team_id: r.get(0)?, name: r.get(1)?, bytes: r.get::<_, i64>(2)? as u64 })
                    })?
                    .collect::<rusqlite::Result<_>>()?;
            }
            Ok(Json(Storage { used, limit: s.config.storage_limit, teams }))
        })
        .await
}
