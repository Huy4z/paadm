//! 统一 JSON 响应结构（见设计方案 6.4 节）。
//!
//! 成功：
//! ```json
//! {"success": true, "data": {...}, "error": null}
//! ```
//! 失败：
//! ```json
//! {"success": false, "data": null, "error": {"code": "...", "message": "..."}}
//! ```
//!
//! 注意：JSON 模式下 stdout 只能出现一个 JSON 对象，日志/诊断信息一律写 stderr。

use serde::Serialize;

use crate::error::AppError;

/// 成功响应：`data` 为业务结果。
#[derive(Serialize)]
struct SuccessResponse<T> {
    success: bool,
    data: T,
    error: Option<()>,
}

/// 失败响应：`data` 显式为 null。
#[derive(Serialize)]
struct FailureResponse {
    success: bool,
    data: Option<()>,
    error: ErrorBody,
}

/// 错误体的稳定字段。
#[derive(Serialize)]
struct ErrorBody {
    code: String,
    message: String,
}

/// 把业务结果以统一 JSON 结构写到 stdout。
pub fn print_success_json<T: Serialize>(data: &T) -> Result<(), AppError> {
    let response = SuccessResponse {
        success: true,
        data,
        error: None,
    };
    let text = serde_json::to_string(&response)?;
    println!("{text}");
    Ok(())
}

/// 把错误以统一 JSON 结构写到 stdout（错误响应本身也必须符合协议）。
pub fn print_failure_json(error: &AppError) {
    let response = FailureResponse {
        success: false,
        data: None,
        error: ErrorBody {
            code: error.code.to_string(),
            message: error.message.clone(),
        },
    };
    // 这里的序列化不可能失败（字段均为字符串/布尔/null）。
    match serde_json::to_string(&response) {
        Ok(text) => println!("{text}"),
        Err(err) => eprintln!("failed to serialize error response: {err}"),
    }
}
