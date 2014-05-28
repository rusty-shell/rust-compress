RUSTC = rustc
RUSTDOC = rustdoc
RUSTFLAGS = -O -D warnings
BUILDDIR = build
COMPRESS = $(BUILDDIR)/$(filter-out %.dylib,\
	      $(shell $(RUSTC) --crate-file-name lib.rs))
APP = $(BUILDDIR)/$(shell $(RUSTC) --crate-file-name main.rs)

LIBTEST = $(BUILDDIR)/test/$(shell $(RUSTC) --crate-file-name --test lib.rs)

all: $(COMPRESS) $(APP)

-include build/compress.d
-include build/app.d
-include build/test/compress.d

$(COMPRESS): lib.rs | $(BUILDDIR)
	$(RUSTC) --out-dir $(@D) $< $(RUSTFLAGS) --dep-info

$(APP): main.rs $(COMPRESS) | $(BUILDDIR)
	$(RUSTC) --out-dir $(@D) $< $(RUSTFLAGS) -L $(BUILDDIR) --dep-info

$(BUILDDIR):
	mkdir -p $@

clean:
	rm -rf build doc

check: test doctest

test: $(LIBTEST)
	$(LIBTEST)

bench: $(LIBTEST)
	$(LIBTEST) --bench

$(LIBTEST): lib.rs
	@mkdir -p $(@D)
	$(RUSTC) $(RUSTFLAGS) --test --out-dir $(@D) lib.rs --dep-info

doctest: $(COMPRESS)
	$(RUSTDOC) --test lib.rs -L build

docs:
	$(RUSTDOC) lib.rs
