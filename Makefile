.DEFAULT_GOAL := default

JUST ?= just

.PHONY: default check fmt fmt-check clippy build build-release release dev dev-macos dev-nixos dev-windows bundle bundle-macos bundle-windows installer-windows smoke smoke-macos smoke-windows dmg gpvim test run ci pack-macos pack-windows

default:
	$(JUST)

check:
	$(JUST) check

fmt:
	$(JUST) fmt

fmt-check:
	$(JUST) fmt-check

clippy:
	$(JUST) clippy

build:
	$(JUST) build

build-release:
	$(JUST) build-release

release:
	$(JUST) build-release

dev:
	$(JUST) dev

dev-macos:
	$(JUST) dev-macos

dev-nixos:
	$(JUST) dev-nixos

dev-windows:
	$(JUST) dev-windows

bundle:
	$(JUST) bundle

bundle-macos:
	$(JUST) bundle-macos

bundle-windows:
	$(JUST) bundle-windows

installer-windows:
	$(JUST) installer-windows

smoke:
	$(JUST) smoke

smoke-macos:
	$(JUST) smoke-macos

smoke-windows:
	$(JUST) smoke-windows

dmg:
	$(JUST) dmg

gpvim:
	$(JUST) gpvim -- $(ARGS)

test:
	$(JUST) test

run:
	$(JUST) run -- $(ARGS)

ci:
	$(JUST) ci

pack-macos:
	$(JUST) pack-macos

pack-windows:
	$(JUST) pack-windows
