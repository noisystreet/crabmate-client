# crabmate-client 构建入口：桌面 / Android 壳、业务 UI、质检与清理。
# 用法：make help
# 壳不 spawn serve；业务 UI 包内加载（prepare-sidecar / prepare-mobile）；API 指向远程 serve。

ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
DESKTOP_ROOT := $(ROOT)/desktop-tauri
TAURI_DIR := $(DESKTOP_ROOT)/src-tauri
MOBILE_ROOT := $(ROOT)/mobile-tauri
MOBILE_TAURI_DIR := $(MOBILE_ROOT)/src-tauri
CONNECT_DIR := $(ROOT)/crates/crabmate-connect
CLIENT_API_DIR := $(ROOT)/crates/crabmate-client-api
TUI_CORE_DIR := $(ROOT)/crates/crabmate-tui-core
TUI_DIR := $(ROOT)/crates/crabmate-tui
FRONTEND_DIR := $(ROOT)/frontend
CARGO ?= cargo

# Android ABI：aarch64 | armv7 | i686 | x86_64（传给 build-apk.sh）
MOBILE_ANDROID_TARGET ?= aarch64
CM_MOBILE_GRADLE_STOP ?= 0
# 1=跳过 trunk（仍 prepare-mobile）；默认会 trunk + 同步包内 UI
CM_MOBILE_SKIP_FRONTEND ?= 0

# 可选：打 desktop deb 时同步进 dist 的 UI 产物目录（默认用本仓 frontend/dist）
# CRABMATE_FRONTEND_DIST ?=

.DEFAULT_GOAL := help

.PHONY: help all \
	prepare-sidecar prepare-mobile sync-connect \
	frontend frontend-release frontend-check frontend-clippy \
	desktop desktop-release desktop-dev desktop-bin-release \
	apk mobile-apk \
	tui tui-release \
	test test-frontend test-tauri test-tui check fmt clippy ktlint-android \
	victauri-e2e victauri-e2e-real e2e-playwright \
	clean clean-desktop clean-mobile clean-connect clean-client-api clean-tui clean-frontend

help:
	@echo "crabmate-client Makefile（仓库根目录执行）"
	@echo ""
	@echo "构建："
	@echo "  make frontend            trunk build（业务 UI → frontend/dist）"
	@echo "  make frontend-release    trunk build --release（需 wasm-opt）"
	@echo "  make prepare-sidecar     同步 connect + frontend dist → desktop-tauri/dist"
	@echo "  make prepare-mobile      同步 connect + frontend dist → mobile-tauri/dist"
	@echo "  make sync-connect        仅同步连接页到 desktop/mobile dist"
	@echo "  make desktop             桌面 debug 安装包（需 cargo-tauri ^2）"
	@echo "  make desktop-release     桌面 release .deb（自动 trunk --release + WASM 体积门禁）"
	@echo "  make desktop-bin-release 仅 release 二进制（不打 deb，较快）"
	@echo "  make desktop-dev         cargo tauri dev（请先自行启动纯 API serve + CORS）"
	@echo "  make tui                 构建 crabmate-tui（远程终端；需外部 serve）"
	@echo "  make tui-release         release 构建 crabmate-tui"
	@echo "  make apk                 Android APK（默认 trunk + 包内 UI）"
	@echo "  make all                 desktop-release"
	@echo ""
	@echo "质检："
	@echo "  make check               bash scripts/check.sh（含 frontend wasm check、ktlint）"
	@echo "  make frontend-check      cargo check --target wasm32-unknown-unknown"
	@echo "  make frontend-clippy     frontend clippy -D warnings"
	@echo "  make test-frontend       frontend：wasm check + lib 单测（与 Tauri 分开）"
	@echo "  make test-tauri          connect + desktop unit tests (--bins) + mobile check（不含 Victauri E2E）"
	@echo "  make test-tui            crabmate-tui-core + crabmate-tui 测试"
	@echo "  make test                test-frontend 然后 test-tauri 然后 test-tui"
	@echo "  make ktlint-android      手改 Android Kotlin ktlint（edu/crabmate）"
	@echo "  make fmt                 七包 cargo fmt（含 client-api / frontend / tui）"
	@echo "  make clippy              七包 clippy -D warnings"
	@echo "  make victauri-e2e        全量 Victauri（需外部 crabmate serve）"
	@echo "  make e2e-playwright      Playwright（需 frontend/dist + serve --with-web）"
	@echo ""
	@echo "清理："
	@echo "  make clean               清理 desktop/mobile/connect/frontend 产物"
	@echo "  make clean-desktop       desktop dist + Tauri target"
	@echo "  make clean-mobile        mobile Tauri target"
	@echo "  make clean-connect       connect target"
	@echo "  make clean-client-api    crabmate-client-api target"
	@echo "  make clean-tui           crabmate-tui* target"
	@echo "  make clean-frontend      frontend dist + target"
	@echo ""
	@echo "变量：CRABMATE_FRONTEND_DIST=…（desktop prepare；默认本仓 frontend/dist）"
	@echo "      CM_PREPARE_SKIP_FRONTEND=1 或 CRABMATE_FRONTEND_DIST=-（不同步 UI）"
	@echo "      CRABMATE_ALLOW_SIBLING_FRONTEND=1（允许回落同级主仓 dist）"
	@echo "      MOBILE_ANDROID_TARGET=aarch64  CM_MOBILE_GRADLE_STOP=1"
	@echo "      CM_MOBILE_SKIP_FRONTEND=1（apk 时跳过 trunk，仍 prepare-mobile）"

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
		echo "错误: 未找到 wasm-opt。trunk build --release 会产出空 .wasm。" >&2; \
		echo "  请执行: cargo install wasm-opt" >&2; \
		exit 1; \
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

prepare-mobile:
	bash "$(MOBILE_ROOT)/scripts/prepare-mobile.sh"

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

# release .deb：beforeBuildCommand → before-desktop-build.sh（trunk --release + 体积门禁）
# CI 可设 CM_PREPARE_SKIP_FRONTEND=1 跳过 UI，仅打壳 stub。
desktop-release: _require_tauri
	cd "$(TAURI_DIR)" && $(CARGO) tauri build

# CI / 快速验证：不跑 bundler
desktop-bin-release: prepare-sidecar
	cd "$(TAURI_DIR)" && $(CARGO) build --release --bin crabmate-desktop

desktop-dev: prepare-sidecar _require_tauri
	cd "$(TAURI_DIR)" && $(CARGO) tauri dev

# --- Android ---

apk mobile-apk:
	MOBILE_ANDROID_TARGET="$(MOBILE_ANDROID_TARGET)" \
		CM_MOBILE_GRADLE_STOP="$(CM_MOBILE_GRADLE_STOP)" \
		CM_MOBILE_SKIP_FRONTEND="$(CM_MOBILE_SKIP_FRONTEND)" \
		bash "$(MOBILE_ROOT)/scripts/build-apk.sh"

# --- 远程终端 ---

tui:
	cd "$(TUI_DIR)" && $(CARGO) build --bin crabmate-tui

tui-release:
	cd "$(TUI_DIR)" && $(CARGO) build --release --bin crabmate-tui

# --- 质检 ---

# frontend：与壳分开；wasm check + lib 单测（跳过需 Server fixtures 的 golden）
test-frontend: frontend-check
	cd "$(FRONTEND_DIR)" && $(CARGO) test --lib -- --nocapture --skip golden_

# Tauri / 壳：connect 逻辑 + desktop 单测；mobile 仅 check（默认无显示则跳过 Victauri）
test-tauri:
	cd "$(CONNECT_DIR)" && $(CARGO) test -- --nocapture
	@mkdir -p "$(DESKTOP_ROOT)/dist"
	@test -f "$(DESKTOP_ROOT)/dist/index.html" || echo '<html></html>' > "$(DESKTOP_ROOT)/dist/index.html"
	@test -f "$(DESKTOP_ROOT)/dist/connect.html" || cp "$(CONNECT_DIR)/assets/connect.html" "$(DESKTOP_ROOT)/dist/connect.html"
	cd "$(TAURI_DIR)" && $(CARGO) test --bins --no-fail-fast
	cd "$(MOBILE_TAURI_DIR)" && $(CARGO) check --tests

test-tui:
	cd "$(TUI_CORE_DIR)" && $(CARGO) test -- --nocapture
	cd "$(TUI_DIR)" && $(CARGO) check

test: test-frontend test-tauri test-tui

check:
	bash "$(ROOT)/scripts/check.sh"

ktlint-android:
	bash "$(ROOT)/scripts/ktlint-android.sh"

fmt:
	cd "$(TAURI_DIR)" && $(CARGO) fmt --all
	cd "$(MOBILE_TAURI_DIR)" && $(CARGO) fmt --all
	cd "$(CLIENT_API_DIR)" && $(CARGO) fmt --all
	cd "$(CONNECT_DIR)" && $(CARGO) fmt --all
	cd "$(TUI_CORE_DIR)" && $(CARGO) fmt --all
	cd "$(TUI_DIR)" && $(CARGO) fmt --all
	cd "$(FRONTEND_DIR)" && $(CARGO) fmt --all

clippy: prepare-sidecar
	cd "$(TAURI_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	cd "$(MOBILE_TAURI_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	cd "$(CLIENT_API_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	cd "$(CONNECT_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	cd "$(TUI_CORE_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	cd "$(TUI_DIR)" && $(CARGO) clippy --all-targets -- -D warnings
	rustup target add wasm32-unknown-unknown 2>/dev/null || true
	cd "$(FRONTEND_DIR)" && $(CARGO) clippy --target wasm32-unknown-unknown --all-targets --all-features -- -D warnings

victauri-e2e:
	./scripts/victauri-e2e.sh all

victauri-e2e-real:
	REAL_LLM_E2E=1 ./scripts/victauri-e2e.sh real_llm

e2e-playwright:
	./scripts/e2e-playwright.sh

# --- 清理 ---

clean: clean-desktop clean-mobile clean-connect clean-client-api clean-tui clean-frontend

clean-desktop:
	rm -rf "$(DESKTOP_ROOT)/dist" "$(DESKTOP_ROOT)/binaries"
	$(CARGO) clean --manifest-path "$(TAURI_DIR)/Cargo.toml"

clean-mobile:
	$(CARGO) clean --manifest-path "$(MOBILE_TAURI_DIR)/Cargo.toml"

clean-connect:
	$(CARGO) clean --manifest-path "$(CONNECT_DIR)/Cargo.toml"

clean-client-api:
	$(CARGO) clean --manifest-path "$(CLIENT_API_DIR)/Cargo.toml"

clean-tui:
	$(CARGO) clean --manifest-path "$(TUI_CORE_DIR)/Cargo.toml"
	$(CARGO) clean --manifest-path "$(TUI_DIR)/Cargo.toml"

clean-frontend:
	rm -rf "$(FRONTEND_DIR)/dist"
	$(CARGO) clean --manifest-path "$(FRONTEND_DIR)/Cargo.toml"
