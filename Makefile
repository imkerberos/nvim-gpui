.DEFAULT_GOAL := check

JUST ?= just

.PHONY: check fmt fmt-check clippy build build-release release bundle dmg gpvim test run ci package-macos package-windows

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
	$(JUST) release

bundle:
	$(JUST) bundle

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

package-macos:
	$(JUST) package-macos

package-windows:
	$(JUST) package-windows
