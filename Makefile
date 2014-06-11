RUSTC = rustc
RUSTDOC = rustdoc
RUSTFLAGS = -D warnings
BUILDDIR = build
COMPRESS = $(BUILDDIR)/$(filter-out %.dylib,\
	      $(shell $(RUSTC) --crate-file-name lib.rs --crate-type=rlib))
APP = $(BUILDDIR)/$(shell $(RUSTC) --crate-file-name main.rs)

LIBTEST = $(BUILDDIR)/test/$(shell $(RUSTC) --crate-file-name --test lib.rs)
LIBBENCH = $(BUILDDIR)/bench/$(shell $(RUSTC) --crate-file-name --test lib.rs)

all: $(COMPRESS) $(APP)

-include build/compress.d
-include build/app.d
-include build/test/compress.d
-include build/bench/compress.d

$(COMPRESS): lib.rs | $(BUILDDIR)
	$(RUSTC) --out-dir $(@D) $< $(RUSTFLAGS) -O --dep-info

$(APP): main.rs $(COMPRESS) | $(BUILDDIR)
	$(RUSTC) --out-dir $(@D) $< $(RUSTFLAGS) -O -L $(BUILDDIR) --dep-info

$(BUILDDIR):
	mkdir -p $@

clean:
	rm -rf build doc

check: test doctest

test: $(LIBTEST)
	$(LIBTEST)

bench: $(LIBBENCH)
	$(LIBBENCH) --bench

$(LIBTEST): lib.rs
	@mkdir -p $(@D)
	$(RUSTC) $(RUSTFLAGS) --test --out-dir $(@D) lib.rs --dep-info

$(LIBBENCH): lib.rs
	@mkdir -p $(@D)
	$(RUSTC) $(RUSTFLAGS) -O --test --out-dir $(@D) lib.rs --dep-info

doctest: $(COMPRESS)
	$(RUSTDOC) --test lib.rs -L build

docs:
	$(RUSTDOC) lib.rs
