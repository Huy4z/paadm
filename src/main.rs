//! paadm 二进制入口。
//!
//! 所有业务逻辑都在库（lib）中，`main` 只负责把 CLI 的退出码透传给操作系统。

fn main() {
    // run() 内部会捕获参数错误、业务错误并输出约定格式；这里只负责返回退出码。
    std::process::exit(paadm::cli::run());
}
