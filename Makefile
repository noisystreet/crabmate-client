# crabmate-client 构建入口：桌面 / Android 壳、业务 UI、质检与清理。
# 用法：make help
# 壳不 spawn serve；业务 UI 默认本仓 frontend/dist（prepare-sidecar / CRABMATE_FRONTEND_DIST）。

ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
DESKTOP_ROOT := $(ROOT)/desktop-tauri
TAURI_DIR := $(DESKTOP_ROOT)/src-tauri
MOBILE_ROOT := $(ROOT)/mobile-tauri
MOBILE_TAURI_DIR := $(MOBILE_ROOT)/src-tauri
CONNECT_DIR := $(ROOT)/crates/crabmate-connect
FRONTEND_DIR := $(ROOT)/frontend
CARGO ?= cargo

# Android ABI：aarch64 | armv7 | i686 | x86_64（传给 build-apk.sh）
MOBILE_ANDROID_TARGET ?= aarch64
CM_MOBILE_GRADLE_STOP ?= 0
# 1=构建本仓/指定 frontend（默认本仓 frontend/；可设 CRABMATE_FRONTEND_DIR）
CM_MOBILE_BUILD_FRONTEND ?= 0

# 可选：打 desktop deb 时同步进 dist 的 UI 产物目录（默认用本仓 frontend/dist）
# CRABMATE_FRONTEND_DIST ?=

.DEFAULT_GOAL := help

.PHONY: help all \
	prepare-sidecar sync-connect \
	frontend frontend-release frontend-check frontend-clippy \
	desktop desktop-release desktop-dev desktop-bin-release \
	apk mobile-apk \
	test check fmt clippy \
	victauri-e2e victauri-e2e-real \
	clean clean-desktop clean-mobile clean-connect clean-frontend

help:
	@echo "crabmate-client Makefile（仓库根目录执行）"
	@echo ""
	@echo "构建："
	@echo "  make frontend            trunk build（业务 UI → frontend/dist）"
	@echo "  make frontend-release    trunk build --release"
	@echo "  make prepare-sidecar     同步 connect/splash（及 frontend dist）到 desktop-tauri/dist"
	@echo "  make sync-connect        同步连接页到 desktop/mobile dist"
	@echo "  make desktop             桌面 debug 安装包（需 cargo-tauri ^2）"
	@echo "  make desktop-release     桌面 release .deb（默认 targets=deb）"
	@echo "  make desktop-bin-release 仅 release 二进制（不打 deb，较快）"
	@echo "  make desktop-dev         cargo tauri dev（请先自行启动 serve）"
	@echo "  make apk                 Android APK（默认不建 frontend）"
	@echo "  make all                 desktop-release"
	@echo ""
	@echo "质检："
	@echo "  make check               bash scripts/check.sh（含 frontend wasm check）"
	@echo "  make frontend-check      cargo check --target wasm32-unknown-unknown"
	@echo "  make frontend-clippy     frontend clippy -D warnings"
	@echo "  make test                connect + desktop cargo test（Victauri 默认跳过）"
	@echo "  make fmt                 四包 cargo fmt（含 frontend）"
	@echo "  make clippy              四包 clippy -D warnings"
	@echo "  make victauri-e2e        全量 Victauri（需外部 crabmate serve）"
	@echo ""
	@echo "清理："
	@echo "  make clean               清理 desktop/mobile/connect/frontend 产物"
	@echo "  make clean-desktop       desktop dist + Tauri target"
	@echo "  make clean-mobile        mobile Tauri target"
	@echo "  make clean-connect       connect target"
	@echo "  make clean-frontend      frontend dist + target"
	@echo ""
	@echo "变量：CRABMATE_FRONTEND_DIST=…（desktop prepare；默认本仓 frontend/dist）"
	@echo "      CM_PREPARE_SKIP_FRONTEND=1 或 CRABMATE_FRONTEND_DIST=-（不同步 UI）"
	@echo "      CRABMATE_ALLOW_SIBLING_FRONTEND=1（允许回落同级主仓 dist）"
	@echo "      MOBILE_ANDROID_TARGET=aarch64  CM_MOBILE_GRADLE_STOP=1"
	@echo "      CM_MOBILE_BUILD_FRONTEND=1（apk 时 trunk 构建本仓 UI）"

# --- 聚合 ---

all: desktop-release

# --- 业务 UI（Leptos / Trunk）---

_require_trunk:
	@command -v trunk >/dev/null 2>&1 || { \
		echo "错误: 未找到 trunk。请执行: cargo install trunk" >&2; \
		exit 1; \
	}

frontend: _require_trunk
	rustup target add wasm32-unknown-unknown 2>/dev/null || true
	cd "$(FRONTEND_DIR)" && unset NO_COLOR && trunk build

frontend-release: _require_trunk
	rustup target add wasm32-unknown-unknown 2>/dev/null || true
	@command -v wasm-opt >/dev/null 2>&1 || { \
		echo "警告: 未找到 wasm-opt。trunk build --release 会产出空 .wasm 文件。建议: cargo install wasm-opt" >&2; \
	}
	cd "$(FRONTEND_DIR)" && unset NO_COLOR && trunk build --release

frontend-check:
	rustup target add wasm32-unknown-unknown 2>/dev/null || true
	cd "$(FRONTEND_DIR)" && $(CARGO) check --target wasm32-unknown-unknown --all-targets

frontend-clippy:
	rustup target add wasm32-unknown-unknown 2>/dev/null || true
	cd "$(FRONTEND_DIR)" && $(CARGO) clippy --target wasm32-unknown-unknown --all-targets --all-features -- -D warnings

# --- 静态资源 ---

prepare-sidecar:
	bash "$(DESKTOP_ROOT)/scripts/prepare-sidecar.sh"

sync-connect:
	bash "$(ROOT)/scripts/sync-tauri-connect-page.sh"

# --- 桌面（Tauri）---

_require_tauri:
	@command -v cargo-tauri >/dev/null 2>&1 || command -v tauri >/dev/null 2>&1 || { \
		echo "错误: 未找到 Tauri CLI。请执行: cargo install tauri-cli --version \"^2\"" >&2; \
		exit 1; \
	}

desktop: prepare-sidecar _require_tauri
	cd "$(TAURI_DIR)" && $(CARGO) tauri build --debug

desktop-release: prepare-sidecar _require_tauri
	cd "$(TAURI_DIR)" && $(CARGO) tauri build

# CI / 快速验证：不跑 bundler
desktop-bin-release: prepare-sidecar
	cd "$(TAURI_DIR)" && $(CARGO) build --release --bin crabmate-desktop

desktop-dev: prepare-sidecar _require_tauri
	cd "$(TAURI_DIR)" && $(CARGO) tauri dev

# --- Android ---

apk mobile-apk: sync-connect
	MOBILE_ANDROID_TARGET="$(MOBILE_ANDROID_TARGET)" \
		CM_MOBILE_GRADLE_STOP="$(CM_MOBILE_GRADLE_STOP)" \
		CM_MOBILE_BUILD_FRONTEND="$(CM_MOBILE_BUILD_FRONTEND)" \
		bash "$(MOBILE_ROOT)/scripts/build-apk.sh"

# --- 质检 ---

check:
	bash "$(ROOT)/scripts/check.sh"

test:
	cd "$(CONNECT_DIR)" && $(CARGO) test
	cd "$(TAURI_DIR)" && $(CARGO) test --no-fail-fast
	cd "$(MOBILE_TAURI_DIR)" && $(CARGO) check --tests

fmt:
	cd "$(TAURI_DIR)" && $(CARGO) fmt --all
	cd "$(MOBILE_TAURI_DIR)" && $(CARGO) fmt --all
	cd "$(CONNECT_DIR)" && $(CARGO) fmt --all
	cd "$(FRONTEND_DIR)" && $(CARGO) fmt --all

clippy: prepare-sidecar
	cd "$(TAURI_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	cd "$(MOBILE_TAURI_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	cd "$(CONNECT_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	rustup target add wasm32-unknown-unknown 2>/dev/null || true
	cd "$(FRONTEND_DIR)" && $(CARGO) clippy --target wasm32-unknown-unknown --all-targets --all-features -- -D warnings

victauri-e2e:
	./scripts/victauri-e2e.sh all

victauri-e2e-real:
	REAL_LLM_E2E=1 ./scripts/victauri-e2e.sh real_llm

# --- 清理 ---

clean: clean-desktop clean-mobile clean-connect clean-frontend

clean-desktop:
	rm -rf "$(DESKTOP_ROOT)/dist" "$(DESKTOP_ROOT)/binaries"
	$(CARGO) clean --manifest-path "$(TAURI_DIR)/Cargo.toml"

clean-mobile:
	$(CARGO) clean --manifest-path "$(MOBILE_TAURI_DIR)/Cargo.toml"

clean-connect:
	$(CARGO) clean --manifest-path "$(CONNECT_DIR)/Cargo.toml"

clean-frontend:
	rm -rf "$(FRONTEND_DIR)/dist"
	$(CARGO) clean --manifest-path "$(FRONTEND_DIR)/Cargo.toml"
