<div align="center">
  <h1>❄️ Snow</h1>
  <p>
    <strong>A high-performance, fault-tolerant programming language built for the modern web.</strong>
  </p>

  <p>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
    <img src="https://img.shields.io/badge/status-experimental-orange.svg" alt="Status">
    <img src="https://img.shields.io/badge/built_with-Rust-dca282.svg" alt="Built with Rust">
    <a href="#"><img src="https://img.shields.io/badge/platform-macos%20%7C%20linux-lightgrey.svg" alt="Platform"></a>
  </p>
</div>

---

**Snow** is a compiled programming language featuring a built-in Actor runtime, designed for building highly concurrent, fault-tolerant networked applications. It bridges the gap between the raw performance of native binaries (via LLVM) and the reliability of the BEAM (Erlang/Elixir) concurrency model.

## 📋 Table of Contents

- [Key Features](#-key-features)
- [Installation](#-installation)
- [Quick Start](#-quick-start)
  - [Initialize a Project](#initialize-a-project)
  - [Basic Actor Example](#basic-actor-example)
  - [HTTP Server Example](#http-server-example)
  - [Closures and Functional Patterns](#closures-and-functional-patterns)
- [CLI Usage](#-cli-usage)
- [Project Structure](#-project-structure)
- [Contributing](#-contributing)
- [License](#-license)

## ✨ Key Features

*   **🎭 Actor Model**: Built-in primitives (`spawn`, `send`, `receive`) for massive concurrency and fault-tolerant supervision trees.
*   **🚀 Native Performance**: Compiles directly to optimized native machine code using **LLVM 21**.
*   **🌐 Zero-Dependency HTTP**: A high-performance, actor-per-connection HTTP server built right into the standard library.
*   **🛡️ "Let it Crash"**: Embrace failure with robust error propagation and supervision strategies.
*   **🛠️ Modern Tooling**: Batteries-included development experience with a built-in package manager, formatter (`fmt`), and Language Server Protocol (`lsp`) support.
*   **🔒 Static Typing**: Strong, static typing with powerful type inference to catch errors at compile time.

## 📦 Installation

Snow is built with Rust. Ensure you have a working Rust toolchain installed, then build from source:

```bash
# Clone the repository
git clone https://github.com/your-username/snow.git
cd snow

# Install the snowc CLI
cargo install --path crates/snowc
```

## 🚀 Quick Start

### Initialize a Project

Create a new project scaffold with a single command:

```bash
snowc init my-project
cd my-project
```

### Basic Actor Example

Snow's concurrency model makes spawning processes cheap and easy.

```snow
actor greeter() do
  receive do
    msg -> println("Received: ${msg}")
  end
end

fn main() do
  # Spawn a new lightweight process
  let pid = spawn(greeter)
  
  # Send a message asynchronously
  send(pid, "Hello from main!")
  
  # Keep main alive briefly (use supervision in real apps)
  Timer.sleep(100)
end
```

### HTTP Server Example

Build robust web services with the built-in, actor-based HTTP server.

```snow
fn handler(request) do
  # Respond with a simple JSON object
  HTTP.response(200, "{\"status\":\"ok\"}")
end

fn main() do
  let r = HTTP.router()
  let r = HTTP.route(r, "/health", handler)
  
  println("Listening on :18080")
  HTTP.serve(r, 18080)
end
```

### Closures and Functional Patterns

Snow supports first-class functions and closures.

```snow
fn main() do
  let n = 5
  let add_n = fn(x :: Int) -> x + n end
  
  println("${add_n(3)}")  # Output: 8
  println("${add_n(10)}") # Output: 15
end
```

## 💻 CLI Usage

The `snowc` tool is your all-in-one companion for Snow development.

| Command | Description |
| :--- | :--- |
| `snowc build` | Compile the current project to a native binary. Use `--opt-level 2` for release builds. |
| `snowc fmt` | specific file or the entire project to standard style. |
| `snowc deps` | Resolve and fetch dependencies defined in `snow.toml`. |
| `snowc repl` | Start an interactive Read-Eval-Print Loop. |
| `snowc lsp` | Start the Language Server (used by VS Code and other editors). |
| `snowc init` | Create a new project in a new directory. |

## 🏗️ Project Structure

The repository is organized as a Cargo workspace containing the compiler and tooling ecosystem:

*   **`crates/snowc`**: The CLI entry point and driver.
*   **`crates/snow-rt`**: The runtime library (Scheduler, HTTP, Actors).
*   **`crates/snow-codegen`**: LLVM IR generation and optimization.
*   **`crates/snow-parser`**: Hand-written recursive descent parser.
*   **`crates/snow-typeck`**: Semantic analysis and type checking.
*   **`crates/snow-pkg`**: Package management logic.
*   **`crates/snow-lsp`**: Language Server Protocol implementation.

## 🤝 Contributing

Contributions are welcome! Please check out the [issues](https://github.com/your-username/snow/issues) or submit a Pull Request.

1.  Fork the Project
2.  Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3.  Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4.  Push to the Branch (`git push origin feature/AmazingFeature`)
5.  Open a Pull Request

## 📄 License

Distributed under the MIT License. See [`LICENSE`](LICENSE) for more information.
