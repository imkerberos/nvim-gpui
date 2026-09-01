.DEFAULT_GOAL := check

JUST ?= just

.PHONY: check fmt fmt-check clippy build release icon bundle gpvim test run ci

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

release:
	$(JUST) release

icon:
	$(JUST) icon

bundle:
	$(JUST) bundle

gpvim:
	$(JUST) gpvim -- $(ARGS)

test:
	$(JUST) test

run:
	$(JUST) run -- $(ARGS)

ci:
	$(JUST) ci
