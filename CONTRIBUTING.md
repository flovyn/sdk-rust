# Contributing to Flovyn Rust SDK

Thank you for your interest in contributing to the Flovyn Rust SDK.

## Development Setup

### Prerequisites

- Rust 1.82+ (install via [rustup](https://rustup.rs/))
- Protocol Buffers compiler (`protoc`)

### Building

```bash
# Clone the repository
git clone https://github.com/flovyn/sdk-rust.git
cd sdk-rust

# Build the entire workspace
cargo build --workspace

# Run tests
cargo test --workspace
```

### Code Quality

Before submitting a PR, ensure your code passes all checks:

```bash
# Format code
cargo fmt

# Run clippy
cargo clippy --workspace -- -D warnings

# Run tests
cargo test --workspace
```

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes
4. Ensure all tests pass
5. Commit your changes with a clear message
6. Push to your fork
7. Open a Pull Request

## Commit Messages

Use clear, descriptive commit messages:

- `feat: add support for workflow timeouts`
- `fix: handle connection timeout in gRPC client`
- `docs: update README with new examples`
- `test: add tests for task cancellation`

## Code Style

- Follow Rust conventions and idioms
- Run `cargo fmt` before committing
- Address all `cargo clippy` warnings
- Add tests for new functionality
- Document public APIs

## Testing

- Unit tests go in the same file as the code they test
- Integration tests go in `tests/`
- Use meaningful test names that describe the behavior being tested

## Questions?

Open an issue if you have questions or need help.
