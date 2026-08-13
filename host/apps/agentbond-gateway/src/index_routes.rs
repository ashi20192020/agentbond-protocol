use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::Value;

use agentbond_db::ReadRepo;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct PageQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub state: Option<String>,
    pub buyer: Option<String>,
    pub provider: Option<String>,
}

fn read_repo(state: &AppState) -> ApiResult<ReadRepo> {
    let db = state.db.as_ref().ok_or_else(|| {
        ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "db_unavailable",
            "database unavailable",
        )
    })?;
    Ok(ReadRepo::new(db.pool().clone()))
}

pub async fn index_status(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let repo = read_repo(&state)?;
    let status = repo.status().await.map_err(map_db)?;
    Ok(Json(
        serde_json::to_value(status).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

pub async fn index_jobs(
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let repo = read_repo(&state)?;
    let page = repo
        .list_jobs(
            q.limit.unwrap_or(20),
            q.cursor.as_deref(),
            q.state.as_deref(),
            q.buyer.as_deref(),
            q.provider.as_deref(),
        )
        .await
        .map_err(map_db)?;
    Ok(Json(
        serde_json::to_value(page).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

pub async fn index_job_history(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let repo = read_repo(&state)?;
    let page = repo
        .job_history(&address, q.limit.unwrap_or(20), q.cursor.as_deref())
        .await
        .map_err(map_db)?;
    Ok(Json(
        serde_json::to_value(page).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

pub async fn index_providers(
    State(state): State<AppState>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let repo = read_repo(&state)?;
    let page = repo
        .list_providers(q.limit.unwrap_or(20), q.cursor.as_deref())
        .await
        .map_err(map_db)?;
    Ok(Json(
        serde_json::to_value(page).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

pub async fn index_provider_activity(
    State(state): State<AppState>,
    Path(address): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<Value>> {
    let repo = read_repo(&state)?;
    let page = repo
        .provider_activity(&address, q.limit.unwrap_or(20), q.cursor.as_deref())
        .await
        .map_err(map_db)?;
    Ok(Json(
        serde_json::to_value(page).map_err(|e| ApiError::bad_request(e.to_string()))?,
    ))
}

fn map_db(err: agentbond_db::DbError) -> ApiError {
    match err {
        agentbond_db::DbError::NotFound(m) => {
            ApiError::new(axum::http::StatusCode::NOT_FOUND, "not_found", m)
        }
        agentbond_db::DbError::Validation(m) => ApiError::bad_request(m),
        agentbond_db::DbError::Sql(_) | agentbond_db::DbError::Migrate(_) => ApiError::new(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "db_unavailable",
            "database unavailable",
        ),
        other => ApiError::bad_request(other.to_string()),
    }
}
