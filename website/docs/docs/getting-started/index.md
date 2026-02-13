---
title: Getting Started
---

# Getting Started

Welcome to Mesh, a programming language designed for expressive, readable concurrency.

## Installation

Mesh is currently in development. To build from source:

```bash
git clone https://github.com/user/mesh
cd mesh
cargo build --release
```

## Hello World

Create a file called `hello.mpl`:

```mesh
fn main() do
  println("Hello, World!")
end
```

Run it with:

```bash
mesh run hello.mpl
```

## Next Steps

Now that you have Mesh installed, explore the language:

- [Language Basics](/docs/language-basics/) -- variables, functions, and control flow
- [Type System](/docs/type-system/) -- structs, enums, and generics
- [Concurrency](/docs/concurrency/) -- actors, message passing, and supervision
