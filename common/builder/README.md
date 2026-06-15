# Builder Derive Macro

A Rust procedural macro that generates type-safe builder patterns using the typestate pattern. This macro automatically creates builder structs with compile-time guarantees that all required fields are set before construction.

## Features

- **Type-Safe Builders**: Generate builder structs with progressive type state
- **Compile-Time Validation**: Ensure all fields are set before calling `build()`
- **Optional Fields**: Mark fields as `#[incomplete]` to create partial configurations
- **Default Values**: Use `#[default(...)]` to provide default field values
- **Smart Wrapping**: Automatic `Arc<dyn Trait>` and `Box<dyn Trait>` wrapping
- **Zero Boilerplate**: Automatic builder generation from struct definitions
- **No Name Conflicts**: Generated builders don't interfere with original structs

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
claude-rust-config-macro = "0.1.0"
```

## Usage

### Basic Example

```rust
use claude_rust_config_macro::Builder;

#[derive(Builder)]
struct ServerConfig {
    host: String,
    port: u16,
    workers: usize,
}

fn main() {
    let config = ServerConfigBuilder::new()
        .with_host("localhost".to_string())
        .with_port(8080)
        .with_workers(4)
        .build();

    println!("Server running on {}:{}", config.host, config.port);
}
```

### Incomplete Fields

Mark optional fields with `#[incomplete]` to create partial configurations:

```rust
#[derive(Builder)]
struct DatabaseConfig {
    host: String,
    port: u16,
    #[incomplete]
    password: String,
}

fn main() {
    // Create incomplete config without password
    let incomplete: DatabaseConfigIncomplete = DatabaseConfigBuilder::new()
        .with_host("localhost".to_string())
        .with_port(5432);

    // Later, complete it with password
    let complete = DatabaseConfigBuilder::new()
        .with_host("localhost".to_string())
        .with_port(5432)
        .with_password("secret".to_string())
        .build();
}
```

### Default Fields

Provide default values for fields using the `#[default(...)]` attribute. Default fields don't participate in typestate progression and can be optionally overridden:

```rust
#[derive(Builder)]
struct WebServerConfig {
    host: String,
    port: u16,
    #[default(4)]
    workers: usize,
    #[default(30)]
    timeout: u64,
}

fn main() {
    // Build without setting default fields - they use their default values
    let config1 = WebServerConfigBuilder::new()
        .with_host("localhost".to_string())
        .with_port(8080)
        .build();

    assert_eq!(config1.workers, 4);
    assert_eq!(config1.timeout, 30);

    // Override default values if needed
    let config2 = WebServerConfigBuilder::new()
        .with_host("localhost".to_string())
        .with_port(8080)
        .with_workers(8)  // Override default
        .build();

    assert_eq!(config2.workers, 8);
}
```

**Key behaviors:**
- Default fields are initialized with their default values in `new()`
- Setters for default fields use mutable consuming pattern (`mut self -> Self`)
- Default fields can be set in any order and are always optional
- Works with complex types: `#[default(vec![])]`, `#[default(None)]`, `#[default(Arc::new(...))]`
- `#[default]` and `#[incomplete]` are mutually exclusive - a field cannot have both attributes

### Smart Wrapping for Trait Objects

The builder automatically wraps trait objects in `Arc` or `Box`, eliminating boilerplate:

```rust
use std::sync::Arc;

trait Logger {
    fn log(&self, msg: &str);
}

struct ConsoleLogger;

impl Logger for ConsoleLogger {
    fn log(&self, msg: &str) {
        println!("{}", msg);
    }
}

#[derive(Builder)]
struct App {
    name: String,
    logger: Arc<dyn Logger>,
}

fn main() {
    // No need for Arc::new()!
    let app = AppBuilder::new()
        .with_name("MyApp".to_string())
        .with_logger(ConsoleLogger)  // Automatically wrapped in Arc
        .build();

    app.logger.log("Application started");
}
```

**Supported patterns:**
- `Arc<dyn Trait>` → Accepts `impl Trait + 'static`
- `Box<dyn Trait>` → Accepts `impl Trait + 'static`
- Multiple bounds: `Arc<dyn Trait + Send + Sync>` → Accepts `impl Trait + Send + Sync + 'static`

**How it works:**
- Builder methods detect `Arc<dyn Trait>` and `Box<dyn Trait>` field types
- Instead of requiring the wrapped type, they accept `impl Trait`
- The value is automatically wrapped with `Arc::new()` or `Box::new()`
- Trait bounds are preserved, with `'static` added when needed

## How It Works

For each struct annotated with `#[derive(Builder)]`, the macro generates:

1. **Builder Struct**: `{StructName}Builder<T1, T2, ...>` with generic type parameters (only for required fields)
2. **Constructor**: `new()` method returning builder with required fields as `()` and default fields initialized
3. **Setter Methods**:
   - Required fields: `with_{field}()` methods that progressively set fields (consuming `self`)
   - Default fields: `with_{field}()` methods that override defaults (mutable consuming `mut self`)
4. **Build Method**: `build()` method available only when all required fields are set
5. **Incomplete Type Alias**: (Optional) `{StructName}Incomplete` when `#[incomplete]` markers exist

### Generated Code Example

```rust
#[derive(Builder)]
struct Config {
    name: String,
    #[incomplete]
    secret: String,
}
```

Generates:

```rust
// Original struct remains unchanged
struct Config {
    name: String,
    secret: String,
}

// Generated builder
struct ConfigBuilder<Name, Secret> {
    name: Name,
    secret: Secret,
}

// Type alias for incomplete state
pub type ConfigIncomplete = ConfigBuilder<String, ()>;

// Constructor
impl ConfigBuilder<(), ()> {
    pub fn new() -> Self { ... }
}

// Builder methods
impl<Name, Secret> ConfigBuilder<Name, Secret> {
    pub fn with_name(self, name: String) -> ConfigBuilder<String, Secret> { ... }
    pub fn with_secret(self, secret: String) -> ConfigBuilder<Name, String> { ... }
}

// Build method (only available when all fields are concrete)
impl ConfigBuilder<String, String> {
    pub fn build(self) -> Config { ... }
}
```

## Typestate Pattern

The generated builders use the **typestate pattern** to enforce correctness at compile time:

- Each field starts as `()` (unit type)
- Calling `with_field()` replaces the generic with the concrete type
- `build()` method is only available when all generics are concrete types
- Incomplete configurations cannot accidentally be used where complete ones are required

## Requirements

- Rust 2021 edition or later
- Works only with structs that have named fields

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.
