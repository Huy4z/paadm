//! 集成测试公共工具：把 paadm 当作真实子进程驱动，使用独立临时数据库。
//!
//! 每个测试用例必须使用独立的临时 SQLite 数据库（`TestDb` 负责创建与清理），
//! 不依赖开发者机器上的真实 `paa.db`，也不依赖系统通知、网络或真实等待。
//!
//! 数据表设计要求所有业务数据都归属于某个用户，因此 `TestDb::new()` 会：
//! 1. 创建临时数据库；
//! 2. 通过 `paadm user create` 创建一个测试用户；
//! 3. 之后所有命令自动带上 `--user-id <测试用户>`。

#![allow(dead_code)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::Value;
use tempfile::TempDir;

/// 一个独立的测试数据库环境（含一个默认测试用户）。
pub struct TestDb {
    // TempDir 在结构体存活期间保持目录存在；drop 时自动清理。
    _dir: TempDir,
    /// 数据库文件路径。
    pub path: PathBuf,
    /// 默认测试用户 ID。
    pub user_id: String,
}

impl Default for TestDb {
    fn default() -> Self {
        Self::new()
    }
}

impl TestDb {
    /// 创建新的临时数据库，并创建默认测试用户。
    pub fn new() -> Self {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path().join("paa.db");
        let user_id = create_user_at(&path, "test", "test-user");
        Self {
            _dir: dir,
            path,
            user_id,
        }
    }

    /// 构造一个只带数据库参数（不含 --user-id）的子进程。
    fn base_command(&self, json: bool) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_paadm"));
        cmd.arg("--db").arg(&self.path);
        if json {
            cmd.arg("--json");
        }
        // 避免外部环境变量干扰测试。
        cmd.env_remove("PAADM_DB_PATH");
        cmd.env_remove("PAADM_DATA_DIR");
        cmd
    }

    /// 构造一个以默认测试用户身份运行的子进程。
    pub fn command(&self, args: &[&str], json: bool) -> Command {
        let mut cmd = self.base_command(json);
        cmd.arg("--user-id").arg(&self.user_id);
        cmd.args(args);
        cmd
    }

    /// 构造一个以指定用户身份运行的子进程。
    pub fn command_for_user(&self, user_id: &str, args: &[&str], json: bool) -> Command {
        let mut cmd = self.base_command(json);
        cmd.arg("--user-id").arg(user_id);
        cmd.args(args);
        cmd
    }

    /// 构造一个不带 --user-id 的子进程（用于测试用户身份缺失场景）。
    pub fn command_without_user(&self, args: &[&str], json: bool) -> Command {
        let mut cmd = self.base_command(json);
        cmd.args(args);
        cmd
    }

    /// 以人类可读模式运行，返回原始输出。
    pub fn raw(&self, args: &[&str]) -> Output {
        self.command(args, false)
            .output()
            .expect("failed to run paadm")
    }

    /// 以 JSON 模式运行，返回原始输出。
    pub fn raw_json(&self, args: &[&str]) -> Output {
        self.command(args, true)
            .output()
            .expect("failed to run paadm")
    }

    /// 以指定用户身份运行 JSON 命令，返回原始输出。
    pub fn raw_json_as(&self, user_id: &str, args: &[&str]) -> Output {
        self.command_for_user(user_id, args, true)
            .output()
            .expect("failed to run paadm")
    }

    /// 以 JSON 模式运行且不带 --user-id，返回原始输出。
    pub fn raw_json_without_user(&self, args: &[&str]) -> Output {
        self.command_without_user(args, true)
            .output()
            .expect("failed to run paadm")
    }

    /// 构造一个 JSON 模式子进程（供并发/锁测试等需要自行控制生命周期的场景）。
    pub fn command_json(&self, args: &[&str]) -> Command {
        self.command(args, true)
    }

    /// 以 JSON 模式运行，并从 stdin 传入内容。
    pub fn raw_json_with_stdin(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command(args, true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn paadm");
        child
            .stdin
            .as_mut()
            .expect("stdin not captured")
            .write_all(stdin.as_bytes())
            .expect("failed to write stdin");
        child.wait_with_output().expect("failed to wait for paadm")
    }

    /// 运行 JSON 命令并断言成功，返回 `data` 字段。
    pub fn ok_json(&self, args: &[&str]) -> Value {
        let output = self.raw_json(args);
        assert_success(&output, args);
        parse_json(&output)["data"].clone()
    }

    /// 以指定用户身份运行 JSON 命令并断言成功，返回 `data` 字段。
    pub fn ok_json_as(&self, user_id: &str, args: &[&str]) -> Value {
        let output = self.raw_json_as(user_id, args);
        assert_success(&output, args);
        parse_json(&output)["data"].clone()
    }

    /// 运行 JSON 命令并断言成功，返回 `data.items` 数组。
    pub fn ok_items(&self, args: &[&str]) -> Vec<Value> {
        let data = self.ok_json(args);
        assert_eq!(data["count"], data["items"].as_array().unwrap().len());
        data["items"]
            .as_array()
            .expect("items must be an array")
            .clone()
    }

    /// 运行 JSON 命令并断言失败，返回 (error JSON, 退出码)。
    pub fn fail_json(&self, args: &[&str]) -> (Value, i32) {
        self.fail_json_output(self.raw_json(args), args)
    }

    /// 以指定用户身份运行 JSON 命令并断言失败。
    pub fn fail_json_as(&self, user_id: &str, args: &[&str]) -> (Value, i32) {
        self.fail_json_output(self.raw_json_as(user_id, args), args)
    }

    fn fail_json_output(&self, output: Output, args: &[&str]) -> (Value, i32) {
        let exit_code = output.status.code().unwrap_or(-1);
        assert_ne!(exit_code, 0, "expected failure, args={args:?}");
        let value = parse_json(&output);
        assert_eq!(
            value["success"],
            Value::Bool(false),
            "expected success=false, got {value}"
        );
        assert!(
            value["error"]["code"].is_string(),
            "expected error.code string, got {value}"
        );
        (value, exit_code)
    }

    /// 在同一个数据库里再创建一个用户，返回其 ID。
    pub fn add_user(&self, provider: &str, external_user_id: &str) -> String {
        create_user_at(&self.path, provider, external_user_id)
    }
}

/// 在指定数据库上创建用户（数据库不存在时会自动初始化），返回用户 ID。
pub fn create_user_at(path: &std::path::Path, provider: &str, external_user_id: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_paadm"))
        .arg("--db")
        .arg(path)
        .args([
            "--json",
            "user",
            "create",
            "--provider",
            provider,
            "--external-user-id",
            external_user_id,
        ])
        .env_remove("PAADM_DB_PATH")
        .env_remove("PAADM_DATA_DIR")
        .output()
        .expect("failed to run paadm user create");
    assert_success(&output, &["user", "create"]);
    let value = parse_json(&output);
    assert_eq!(value["success"], Value::Bool(true));
    id_of(&value["data"])
}

/// 断言子进程成功退出，并在失败时打印 stdout/stderr 便于定位。
pub fn assert_success(output: &Output, args: &[&str]) {
    assert!(
        output.status.success(),
        "command failed: args={args:?} exit_code={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 解析 stdout 为 JSON（stdout 只允许一个 JSON 对象）。
pub fn parse_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|err| {
        panic!(
            "stdout is not valid JSON: {err}\nstdout={stdout}\nstderr={}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// 断言错误码。
pub fn assert_error_code(error: &Value, expected: &str) {
    assert_eq!(
        error["error"]["code"], expected,
        "unexpected error code: {error}"
    );
}

/// 从 create/get 返回的 data 中取 id。
pub fn id_of(data: &Value) -> String {
    data["id"]
        .as_str()
        .unwrap_or_else(|| panic!("missing id in {data}"))
        .to_string()
}
