# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Chimera-md is a Markdown-aware web server written in Rust that serves HTML versions of Markdown files transparently. It combines static file serving with dynamic Markdown processing, full-text search, and live file watching.

## Architecture

The codebase follows a modular architecture with these key components:

- **main.rs** - Entry point with Axum web server setup, routing, and middleware
- **toml_config.rs** - Configuration management from TOML files  
- **html_generator.rs** - Converts Markdown to HTML using Tera templates
- **full_text_index.rs** - Tantivy-based search indexing and querying
- **file_manager.rs** - File system monitoring and peer discovery
- **document_scraper.rs** - Markdown parsing with pulldown-cmark
- **result_cache.rs** - In-memory HTML result caching
- **image_size_cache.rs** - Image dimension caching for layout stability

The server serves content from two main roots:
- `/home/*` - User Markdown documents and assets
- `/*` - Static web files (CSS, favicon, etc.)

## Common Development Commands

### Building and Running
```bash
# Standard Rust build
cargo build
cargo run

# Run with example configuration
cargo run -- --config-file=example/chimera.toml

# Development with auto-reload using bacon
bacon example

# Build Docker image
docker build -t chimera-md .

# Multi-platform Docker build
./docker/multi-build.sh
```

### Development Tools
```bash
# Type checking
cargo check
cargo clippy

# Testing  
cargo test

# Documentation
cargo doc --no-deps --open

# Development server with file watching
bacon example
```

### Docker Operations
```bash
# Run with Docker Compose
docker-compose -f docker/compose.yaml up

# Build and run locally
docker build -t chimera-md .
docker run -p 8080:8080 -v ./example:/data chimera-md
```

## Configuration

The server uses TOML configuration files (typically `chimera.toml`). Key settings:

- `chimera_root` - Base directory for all server files
- `site_title` - Website title
- `port` - Server port (default 8080)
- `generate_index` - Auto-generate directory indexes
- `image_size_file` - Path to image dimensions cache
- `[menu]` - Navigation menu items  
- `[redirects]` - URL redirect mappings
- `[cache_control]` - HTTP cache headers by content type

## Key Architectural Patterns

- **Async/Await**: Built on Tokio runtime with async file I/O
- **Caching**: Multi-layer caching (result cache, image size cache)
- **File Watching**: Real-time updates using async-watcher
- **Template System**: Tera templates for HTML generation
- **Middleware**: Custom middleware for headers, logging, timing
- **Error Handling**: Custom ChimeraError type with proper propagation

## Important File Paths

Configuration and content paths are relative to `chimera_root`:
- `home/` - Markdown documents
- `www/` - Static web assets  
- `template/` - User-customizable Tera templates
- `template-internal/` - Built-in templates
- `search/` - Full-text index storage
- `log/` - Application logs

## Testing

The project includes an example configuration in `example/` with:
- Sample Markdown files and assets
- Test templates
- Configuration demonstrating all features
- Image size cache example

Use `bacon example` to run the development server with the example content.