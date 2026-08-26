# Allocator safety

**RustyCSV ships with no bundled memory allocator, and you should think hard
before enabling one.**

Since v0.4.5 the `mimalloc` cargo feature is opt-in rather than default. This
page explains why, what it costs you, and when it is safe to turn back on.

## Why the default changed

A NIF is a shared library loaded into a BEAM that may already host other NIFs.
When a Rust NIF sets `#[global_allocator]` to a statically linked allocator,
that allocator is private to *that* `.so` — its arenas, its free lists, and (for
mimalloc) its page map are not shared with any other NIF.

On macOS, mimalloc also stores the per-thread default heap in a **fixed TLS
slot** (offset `0x360` from the thread pointer). That slot is process-global.
So when two NIFs each bundle mimalloc:

- they **share** the TLS slot that names the current thread's heap, but
- they each keep a **private** page map for looking blocks up.

A block allocated while the slot held the other copy's heap is therefore
invisible to this copy's page map. `mi_realloc` then takes this path:

```c
size_t size = _mi_usable_size(p);        // page-map miss -> 0
void*  newp = mi_heap_malloc(heap, newsize);
memcpy(newp, p, min(newsize, size));     // min(newsize, 0) == 0 bytes
mi_free(p);                              // original freed anyway
```

It copies **nothing**, frees the original, and hands back fresh, uninitialised
memory — which reads as zeroes on a fresh page. Every live value in that block
is silently destroyed. No error, no crash at the point of damage.

### How RustyCSV is involved

The crashes that exposed this were observed in [RustyJson][rustyjson], not here:
a small 3-key root object grew its key vector from capacity 2 to 4, the
zero-byte `realloc` destroyed `keys[0]` and `keys[1]`, and those words reached
`enif_make_map_from_arrays` as `THE_NON_VALUE`. That function validates nothing,
so the VM died inside `erts_cmp` dereferencing address `0xfffffffffffffffe`.
Three production dev-server crashes on 2026-08-25, then reproduced on demand.

RustyCSV is the *other half* of that pair. It bundled mimalloc by default
through 0.4.4, and it was the second allocator-bundling NIF in the VM where the
crashes happened — the reproduction was built specifically against `rusty_csv`
0.4.4.

This matters more than it might look. The corruption is **symmetric and
non-local**: either copy of mimalloc can be handed the other's block, and the
damage lands in whichever library happens to reallocate next. Nothing about
RustyCSV's own parsing or encoding code makes it the safe side of the pair, and
no amount of validation inside RustyCSV could detect it — the corruption happens
in the allocator, below the library. Shipping a bundled allocator by default was
enough to endanger every other NIF in the VM, so RustyCSV stopped.

Full investigation, including the disassembly and the reproduction:
[`BEAM_SEGFAULT_INVESTIGATION.md`][investigation] in the RustyJson repository.

## What it costs to leave the allocator off

Measured on an M1 Pro (10 cores), OTP 29 / Elixir 1.20.3, release builds of the
same commit differing only in the `mimalloc` cargo feature. Four interleaved A/B
pairs, seven rounds each, reduced by **minimum** per operation — the host was
under heavy concurrent load, and contention only ever adds time, so the fastest
observed round is the closest estimate of undisturbed cost.

| operation | no allocator | mimalloc | mimalloc gain |
|---|---|---|---|
| decode 25 B, one row | 2.09 µs | 1.86 µs | 1.12× |
| decode 333 KB unquoted, 10k rows | 1.32 ms | 1.23 ms | 1.07× |
| decode 947 KB quoted, 10k rows | 3.93 ms | 3.73 ms | 1.05× |
| decode 6.8 MB mixed, 100k rows | 22.5 ms | 20.8 ms | 1.08× |
| decode 6.8 MB mixed, `strategy: :parallel` | 20.4 ms | 19.8 ms | 1.03× |
| decode 947 KB to maps, `headers: true` | 4.50 ms | 3.72 ms | 1.21× |
| stream 6.8 MB in 64 KB chunks | 67.3 ms | 50.1 ms | **1.34×** |
| encode 10k quoted rows | 2.42 ms | 2.45 ms | 0.99× |
| encode 100k mixed rows | 25.0 ms | 25.3 ms | 0.99× |
| encode 100k mixed rows, `strategy: :parallel` | 43.3 ms | 28.7 ms | **1.51×** |
| **aggregate** | **191 ms** | **156 ms** | **1.22×** |

So dropping the bundled allocator costs about **22% across this mixed
workload** — but the cost is not spread evenly, and the average is the least
useful number in the table:

- **The multi-threaded paths pay for it.** `:parallel` encoding is 1.5× slower
  and chunked streaming 1.34× slower without mimalloc. These are the paths that
  allocate hardest from several threads at once, and per-thread arenas are
  precisely what mimalloc is good at.
- **Single-threaded encoding pays nothing.** Both serial encode figures land
  within 1%, which is inside the noise here. The encoder writes into a small
  number of large buffers, so the allocator is barely on its path.
- **Single-threaded decoding pays 3–8%**, rising to 21% for `headers: true`,
  which allocates a map per row.

If your workload is serial `dump_to_iodata/1`, this release costs you nothing
measurable. If it is `strategy: :parallel` exports or high-volume streaming, it
costs you real throughput — and that is exactly the case where you should work
through the audit below rather than simply switching the allocator back on.

Note on the measurement: RustyJson reported ~20% on decode with its largest
gains on small payloads. RustyCSV's profile is different — flat on serial
encode, concentrated on the threaded paths — so these numbers are RustyCSV's
own, not carried over.

## When it is safe to enable

Enable a bundled allocator only if **RustyCSV is the only NIF in your VM that
bundles one**. Check every Rust NIF you load:

```sh
for so in _build/dev/lib/*/priv/native/*.so; do
  n=$(strings -a "$so" | grep -c "^mimalloc: error: $")
  [ "$n" -gt 0 ] && echo "bundles mimalloc: $so"
done
```

If that prints nothing other than RustyCSV, you are clear. `rustyjson` is the
other Elixir NIF known to offer a bundled mimalloc, and it has the same opt-in
default since its 0.4.1. Re-run the check whenever you add a Rust dependency —
the failure mode is silent.

Then:

```sh
RUSTYCSV_ALLOCATOR=mimalloc FORCE_RUSTYCSV_BUILD=1 mix compile
```

`mimalloc` is the only accepted value; it is the only allocator RustyCSV carries
as a cargo feature. This requires a local build — the published precompiled
artifacts never bundle an allocator.

The general rule holds beyond mimalloc: two NIFs bundling the *same* allocator
can collide, and the consequences are silent heap corruption. Only one NIF per
VM should override the global allocator.

### A note for musl targets

`native/rustycsv/Cargo.toml` still selects `mimalloc/local_dynamic_tls` for
`target_env = "musl"`. That entry is now only reachable through the opt-in
feature, so musl builds — Alpine images in particular — fall back to musl's own
`mallocng` by default. `mallocng` is markedly weaker than mimalloc under
concurrent allocation, so RustyCSV's `:parallel` strategies are expected to lose
more there than the macOS figures above show. That trade is deliberate: a
predictable slowdown beats an unpredictable VM crash. If a musl deployment needs
the throughput back and RustyCSV is the only allocator-bundling NIF in the
image, the opt-in build is the supported way to get it.

## Things that do not fix this

- **`mimalloc/local_dynamic_tls`** — verified not to help. It changes the
  `__thread` TLS model; mimalloc's macOS `__builtin_thread_pointer()` fast path
  is independent of it and the fixed-slot accesses remain. RustyCSV kept this
  feature for musl for unrelated reasons; it does not make bundling safe.
- **Validation inside RustyCSV** — cannot work. The corruption happens below the
  library, in the allocator, and by the time a term is observably wrong the
  bytes backing it are already gone.
- **Rebuilding for a different NIF ABI** — verified irrelevant. The corruption
  reproduces identically on NIF 2.15, 2.17 and 2.18, and disappears on all three
  when the allocator is removed.

## Regression guard

`test/allocator_isolation_test.exs` fails the build if a default artifact ever
links mimalloc again. It is deliberately a build-shape assertion rather than a
functional test, because no functional test can detect this class of bug — the
corruption is silent until the VM dies somewhere unrelated.

To audit a released artifact directly:

```sh
nm -a priv/native/rustycsv.so | grep -c ' _mi_\| mi_'                          # expect 0
grep -c -a -e 'mimalloc: error: ' -e 'unable to extend the page map' \
  priv/native/rustycsv.so                                                      # expect 0
```

[rustyjson]: https://github.com/jeffhuen/RustyJson
[investigation]: https://github.com/jeffhuen/RustyJson/blob/main/docs/BEAM_SEGFAULT_INVESTIGATION.md
