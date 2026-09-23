//! Usage statistics commands

use crate::error::AppError;
use crate::services::usage_stats::*;
use crate::store::AppState;
use tauri::State;

/// Get the usage summary
#[tauri::command]
pub fn get_usage_summary(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
) -> Result<UsageSummary, AppError> {
    state.db.get_usage_summary(start_date, end_date)
}

/// Get the daily trend
#[tauri::command]
pub fn get_usage_trends(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
) -> Result<Vec<DailyStats>, AppError> {
    state.db.get_daily_trends(start_date, end_date)
}

/// Per-account stats over a time range
#[tauri::command]
pub fn get_provider_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
) -> Result<Vec<ProviderStats>, AppError> {
    state.db.get_provider_stats(start_date, end_date)
}

/// Per-model stats over a time range
#[tauri::command]
pub fn get_model_stats(
    state: State<'_, AppState>,
    start_date: Option<i64>,
    end_date: Option<i64>,
) -> Result<Vec<ModelStats>, AppError> {
    state.db.get_model_stats(start_date, end_date)
}

/// Account switches, newest first
#[tauri::command]
pub fn get_account_switches(
    state: State<'_, AppState>,
    app_type: Option<String>,
    start_date: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<crate::database::AccountSwitch>, AppError> {
    state
        .db
        .get_account_switches(app_type.as_deref(), start_date, limit.unwrap_or(200))
}

/// Get the request log list
#[tauri::command]
pub fn get_request_logs(
    state: State<'_, AppState>,
    filters: LogFilters,
    page: u32,
    page_size: u32,
) -> Result<PaginatedLogs, AppError> {
    state.db.get_request_logs(&filters, page, page_size)
}

/// Get a single request's details
#[tauri::command]
pub fn get_request_detail(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Option<RequestLogDetail>, AppError> {
    state.db.get_request_detail(&request_id)
}

/// Get the model pricing list
#[tauri::command]
pub fn get_model_pricing(state: State<'_, AppState>) -> Result<Vec<ModelPricingInfo>, AppError> {
    log::info!("Getting the model pricing list");
    state.db.ensure_model_pricing_seeded()?;

    let db = state.db.clone();
    let conn = crate::database::lock_conn!(db.conn);

    // Check that the table exists
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_pricing'",
            [],
            |row| row.get::<_, i64>(0).map(|count| count > 0),
        )
        .unwrap_or(false);

    if !table_exists {
        log::error!("The model_pricing table does not exist; the app may need a restart to run the database migration");
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
         FROM model_pricing
         ORDER BY display_name",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(ModelPricingInfo {
            model_id: row.get(0)?,
            display_name: row.get(1)?,
            input_cost_per_million: row.get(2)?,
            output_cost_per_million: row.get(3)?,
            cache_read_cost_per_million: row.get(4)?,
            cache_creation_cost_per_million: row.get(5)?,
        })
    })?;

    let mut pricing = Vec::new();
    for row in rows {
        pricing.push(row?);
    }

    log::info!("Loaded {} model pricing entries", pricing.len());
    Ok(pricing)
}

/// Update model pricing
#[tauri::command]
pub fn update_model_pricing(
    state: State<'_, AppState>,
    model_id: String,
    display_name: String,
    input_cost: String,
    output_cost: String,
    cache_read_cost: String,
    cache_creation_cost: String,
) -> Result<(), AppError> {
    let db = state.db.clone();
    let conn = crate::database::lock_conn!(db.conn);

    conn.execute(
        "INSERT OR REPLACE INTO model_pricing (
            model_id, display_name, input_cost_per_million, output_cost_per_million,
            cache_read_cost_per_million, cache_creation_cost_per_million
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            model_id,
            display_name,
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost
        ],
    )
    .map_err(|e| AppError::Database(format!("Failed to update model pricing: {e}")))?;

    Ok(())
}

/// Check a provider's usage limits
#[tauri::command]
pub fn check_provider_limits(
    state: State<'_, AppState>,
    provider_id: String,
    app_type: String,
) -> Result<crate::services::usage_stats::ProviderLimitStatus, AppError> {
    state.db.check_provider_limits(&provider_id, &app_type)
}

/// Delete model pricing
#[tauri::command]
pub fn delete_model_pricing(state: State<'_, AppState>, model_id: String) -> Result<(), AppError> {
    let db = state.db.clone();
    let conn = crate::database::lock_conn!(db.conn);

    conn.execute(
        "DELETE FROM model_pricing WHERE model_id = ?1",
        rusqlite::params![model_id],
    )
    .map_err(|e| AppError::Database(format!("Failed to delete model pricing: {e}")))?;

    log::info!("Deleted model pricing: {model_id}");
    Ok(())
}

/// Model pricing info
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricingInfo {
    pub model_id: String,
    pub display_name: String,
    pub input_cost_per_million: String,
    pub output_cost_per_million: String,
    pub cache_read_cost_per_million: String,
    pub cache_creation_cost_per_million: String,
}
