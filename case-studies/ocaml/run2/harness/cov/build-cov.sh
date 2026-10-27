#!/bin/bash
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

# build-cov.sh <src-dir> <build-dir> <cfg>      (cfg = closure | flambda)
#
# Builds the OCaml compiler from <src-dir> with lambda/, middle_end/ and
# asmcomp/ (incl. asmcomp/$ARCH/*.ml and the cvt_emit-generated emit.ml from
# asmcomp/$ARCH/emit.mlp) instrumented with bisect_ppx, installed into
# <build-dir>/<cfg>/inst.  Also provides <build-dir>/tools/bin/bisect-ppx-report.
#
# Coverage file names recorded in the data are the ABSOLUTE paths of the files
# in <src-dir> (as given, made absolute with `cd <src-dir> && pwd`, no symlink
# resolution), e.g. /path/to/src/asmcomp/cmmgen.ml, and
# /path/to/src/asmcomp/amd64/emit.mlp for the emitter (offsets remapped to the
# .mlp; cvt_emit preserves lines, columns inside `...` templates are
# approximate).
#
# The instrumented compiler writes coverage only when BISECT_FILE is set
# (files $BISECT_FILE<random>.coverage are written at exit).  BISECT_SILENT=ERR
# shows runtime write errors on stderr (silent by default).
#
# Re-runnable/incremental: the source is mirrored with rsync into
# <build-dir>/<cfg>/pristine; only changed files are recopied / re-instrumented
# in <build-dir>/<cfg>/src, then make + make install are re-run.
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: $0 <src-dir> <build-dir> <closure|flambda>" >&2; exit 2
fi
SRC=$(cd "$1" && pwd)
mkdir -p "$2"
BUILD=$(cd "$2" && pwd)
CFG=$3
case "$CFG" in
  closure) CONF_ARGS=() ;;
  flambda) CONF_ARGS=(--enable-flambda --enable-flambda-invariants) ;;
  *) echo "cfg must be closure or flambda" >&2; exit 2 ;;
esac
JOBS=${JOBS:-16}

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HOSTBIN=${HOSTBIN:-/agent-run/work/ref/boot/bin}
export PATH="$HOSTBIN:$PATH"
TOOLS=$HERE/tools
WS=$TOOLS/ws
D=$BUILD/$CFG
P=$D/pristine
S=$D/src
LOG=$D/logs
mkdir -p "$D" "$LOG"
unset BISECT_FILE   # the vendored runtime writes nothing without it
t0=$(date +%s)
say() { echo "[build-cov $(( $(date +%s) - t0 ))s] $*"; }

# ---------------------------------------------------------------- tools ----
ensure_tools() {
  local dl=$TOOLS/dl
  if [ ! -f "$WS/.extracted" ]; then
    say "downloading ppxlib/bisect_ppx and deps"
    mkdir -p "$dl" "$WS"
    get() { [ -s "$dl/$1.tar.gz" ] || curl -sfL -o "$dl/$1.tar.gz" "$2"; }
    get ppxlib https://github.com/ocaml-ppx/ppxlib/archive/refs/tags/0.37.0.tar.gz
    # bisect_ppx 2.8.3 + PR #448 (ppxlib >= 0.36 support), commit 7061d64
    get bisect_ppx https://github.com/patricoferris/bisect_ppx/archive/7061d643ff492b0045796357ee6917ded21fb1f0.tar.gz
    get cmdliner https://github.com/dbuenzli/cmdliner/archive/refs/tags/v1.3.0.tar.gz
    get sexplib0 https://github.com/janestreet/sexplib0/archive/refs/tags/v0.17.0.tar.gz
    get ppx_derivers https://github.com/ocaml-ppx/ppx_derivers/archive/refs/tags/1.2.1.tar.gz
    get stdlib-shims https://github.com/ocaml/stdlib-shims/archive/refs/tags/0.3.0.tar.gz
    get ocaml-compiler-libs https://github.com/janestreet/ocaml-compiler-libs/archive/refs/tags/v0.17.0.tar.gz
    for f in "$dl"/*.tar.gz; do
      n=$(basename "$f" .tar.gz); rm -rf "${WS:?}/$n"; mkdir -p "$WS/$n"
      tar xzf "$f" -C "$WS/$n" --strip-components=1
    done
    rm -rf "$WS/bisect_ppx/src/ppx/js"     # melange/js bits, not needed
    echo '(lang dune 3.0)' > "$WS/dune-project"
    touch "$WS/.extracted"
  fi
  mkdir -p "$WS/driver"
  cmp -s "$HERE/driver/dune" "$WS/driver/dune" || cp "$HERE/driver/dune" "$WS/driver/dune"
  cmp -s "$HERE/driver/driver.ml" "$WS/driver/driver.ml" || cp "$HERE/driver/driver.ml" "$WS/driver/driver.ml"
  (cd "$WS" && dune build --root . driver/driver.exe @bisect_ppx/install 2>&1 \
     | grep -v -e 'Cache directories' -e 'disabling cache' -e 'Hint: Make sure' || true)
  DRIVER=$WS/_build/default/driver/driver.exe
  REPORT=$WS/_build/install/default/bin/bisect-ppx-report
  [ -x "$DRIVER" ] && [ -x "$REPORT" ] || { echo "tool build failed" >&2; exit 1; }
  mkdir -p "$BUILD/tools/bin"
  ln -sf "$REPORT" "$BUILD/tools/bin/bisect-ppx-report"
  ln -sf "$DRIVER" "$BUILD/tools/bin/cov-ppx-driver"
}
# closure and flambda builds run in parallel: one tool setup at a time (a fresh
# checkout raced: one extracted while the other ran dune)
mkdir -p "$TOOLS"
exec 9>"$TOOLS/.lock"
flock 9
ensure_tools
flock -u 9
say "tools ok: $BUILD/tools/bin/bisect-ppx-report"

# Anything that changes the instrumentation invalidates all instrumented files.
STAMP=$( (sha256sum "$DRIVER" "$HERE"/rt/* "${BASH_SOURCE[0]}" | awk '{print $1}') | sha256sum | cut -c1-16)

# ----------------------------------------------------------------- sync ----
FIRST=0
[ -f "$S/.cov-synced" ] || FIRST=1
CHANGED=$D/changed.txt
rsync -a --delete -i --exclude=/.git --exclude=/_build --exclude=/_opam \
  --out-format='%i %n' "$SRC"/ "$P"/ > "$D/rsync.out"
if [ $FIRST = 1 ]; then
  say "initial copy of source tree"
  rm -rf "$S"; mkdir -p "$S"
  cp -r "$P"/. "$S"/
  : > "$CHANGED"
else
  grep -E '^>f' "$D/rsync.out" | cut -d' ' -f2- > "$CHANGED" || true
  { grep -E '^\*deleting' "$D/rsync.out" || true; } | sed 's/^\*deleting *//' | while read -r f; do
    rm -f "$S/$f"; done
  while read -r f; do
    mkdir -p "$(dirname "$S/$f")"; cp "$P/$f" "$S/$f"   # fresh mtime
  done < "$CHANGED"
  say "$(wc -l < "$CHANGED") changed file(s)"
fi

# ------------------------------------------------------------ configure ----
CONF_SIG="${CONF_ARGS[*]:-} prefix=$D/inst"
if [ ! -f "$S/config.status" ] || [ "$(cat "$D/.configure-sig" 2>/dev/null)" != "$CONF_SIG" ] \
   || grep -qxE 'configure|Makefile\.config\.in|Makefile\.build_config\.in' "$CHANGED"; then
  say "configure ${CONF_ARGS[*]:-}"
  (cd "$S" && ./configure --prefix="$D/inst" "${CONF_ARGS[@]}") > "$LOG/configure.log" 2>&1 \
    || { echo "configure failed, see $LOG/configure.log" >&2; exit 1; }
  echo "$CONF_SIG" > "$D/.configure-sig"
fi
ARCH=$(sed -n 's/^ARCH *= *//p' "$S/Makefile.config" | head -1)
[ -n "$ARCH" ] || { echo "cannot determine ARCH" >&2; exit 1; }

# ----------------------------------------------------------- instrument ----
FULL=0
[ "$(cat "$D/.cov-stamp" 2>/dev/null)" = "$STAMP" ] || FULL=1
INSTR_LIST=$D/instrumented.txt
(cd "$P" && { ls lambda/*.ml asmcomp/*.ml "asmcomp/$ARCH"/*.ml 2>/dev/null
              find middle_end -name '*.ml'; } | sort) > "$INSTR_LIST"
if [ $FULL = 1 ] || [ $FIRST = 1 ]; then
  TODO=$INSTR_LIST
else
  TODO=$D/todo.txt
  grep -Fxf "$CHANGED" "$INSTR_LIST" > "$TODO" || true
fi

instrument_one() {   # $1 = relative path; env: DRIVER P S SRC
  local f=$1 tmp
  tmp=$(mktemp "$S/$f.covtmp.XXXXXX")
  if ! "$DRIVER" -impl "$P/$f" -loc-filename "$SRC/$f" -o "$tmp.raw" 2> "$tmp.err"; then
    echo "instrumentation failed: $f" >&2; cat "$tmp.err" >&2; rm -f "$tmp" "$tmp.raw" "$tmp.err"; return 1
  fi
  { echo '[@@@ocaml.warning "-a"]'
    sed 's/\bBisect\.Runtime\.register_file\b/Ocaml_cov_bisect_runtime.register_file/' "$tmp.raw"; } > "$tmp"
  rm -f "$tmp.raw" "$tmp.err"
  if cmp -s "$tmp" "$S/$f"; then rm -f "$tmp"; else mv -f "$tmp" "$S/$f"; fi
}
export -f instrument_one
export DRIVER P S SRC
n=$(wc -l < "$TODO")
if [ "$n" -gt 0 ]; then
  say "instrumenting $n file(s)"
  xargs -a "$TODO" -P "$JOBS" -I{} bash -c 'instrument_one "$1"' _ {}
fi

# emit.mlp -> emit.ml (host-built cvt_emit) -> instrumented asmcomp/$ARCH/emit.ml.cov
MLP=asmcomp/$ARCH/emit.mlp
HT=$D/hosttools
mkdir -p "$HT"
if [ ! -x "$HT/cvt_emit" ] || [ "$P/tools/cvt_emit.mll" -nt "$HT/cvt_emit" ]; then
  ocamllex -q "$P/tools/cvt_emit.mll" -o "$HT/cvt_emit.ml"
  (cd "$HT" && ocamlopt -o cvt_emit cvt_emit.ml) > "$LOG/cvt_emit.log" 2>&1
  rm -f "$S/asmcomp/$ARCH/emit.ml.cov"
fi
if [ $FULL = 1 ] || [ ! -f "$S/asmcomp/$ARCH/emit.ml.cov" ] || grep -qxF "$MLP" "$CHANGED"; then
  say "instrumenting $MLP"
  "$HT/cvt_emit" < "$P/$MLP" > "$HT/emit.ml"
  "$DRIVER" -impl "$HT/emit.ml" -loc-filename "$SRC/$MLP" -cov-remap-mlp "$P/$MLP" -o "$HT/emit.inst.ml"
  { echo '[@@@ocaml.warning "-a"]'
    sed 's/\bBisect\.Runtime\.register_file\b/Ocaml_cov_bisect_runtime.register_file/' "$HT/emit.inst.ml"; } > "$HT/emit.ml.cov"
  cmp -s "$HT/emit.ml.cov" "$S/asmcomp/$ARCH/emit.ml.cov" || cp "$HT/emit.ml.cov" "$S/asmcomp/$ARCH/emit.ml.cov"
fi

# ------------------------------------------------- runtime + build glue ----
for f in "$HERE"/rt/ocaml_cov_bisect_*.ml*; do
  cmp -s "$f" "$S/utils/$(basename "$f")" || cp "$f" "$S/utils/"
done

# Makefile: add runtime modules to utils_SOURCES (ocamlcommon), and make
# asmcomp/emit.ml a copy of the pre-instrumented emitter.
perl -0pe '
  s{^(utils_SOURCES = \$\(addprefix utils/, \\\n)}{$1  ocaml_cov_bisect_common.mli ocaml_cov_bisect_common.ml \\\n  ocaml_cov_bisect_runtime.mli ocaml_cov_bisect_runtime.ml \\\n}m or die "patch1";
  s{^asmcomp/emit\.ml: asmcomp/\$\(ARCH\)/emit\.mlp \$\(cvt_emit\)\n(?:\t.*\n)+}{asmcomp/emit.ml: asmcomp/\$(ARCH)/emit.ml.cov\n\tcp \$< \$\@\n}m or die "patch2";
' "$P/Makefile" > "$D/Makefile.patched"
cmp -s "$D/Makefile.patched" "$S/Makefile" || cp "$D/Makefile.patched" "$S/Makefile"

# .depend: instrumented modules depend on the runtime.
{
  cat "$P/.depend"
  R=utils/ocaml_cov_bisect_runtime C=utils/ocaml_cov_bisect_common
  echo "# ---- coverage build additions ----"
  echo "$C.cmi :"
  echo "$C.cmo : $C.cmi"
  echo "$C.cmx : $C.cmi"
  echo "$R.cmi :"
  echo "$R.cmo : $C.cmi $R.cmi"
  echo "$R.cmx : $C.cmi $C.cmx $R.cmi"
  { sed -e "s#^asmcomp/$ARCH/#asmcomp/#" "$INSTR_LIST"; echo asmcomp/emit.ml; } | sort -u |
  while read -r f; do
    m=${f%.ml}
    echo "$m.cmo $m.cmx : $R.cmi"
    echo "$m.cmx : $R.cmx"
  done
} > "$D/depend.patched"
cmp -s "$D/depend.patched" "$S/.depend" || cp "$D/depend.patched" "$S/.depend"

echo "$STAMP" > "$D/.cov-stamp"
touch "$S/.cov-synced"

# ---------------------------------------------------------------- build ----
say "make -j$JOBS (log: $LOG/make.log)"
(cd "$S" && make -j"$JOBS") > "$LOG/make.log" 2>&1 || {
  echo "make failed, tail of $LOG/make.log:" >&2; tail -40 "$LOG/make.log" >&2; exit 1; }
say "make install (log: $LOG/install.log)"
(cd "$S" && make install) > "$LOG/install.log" 2>&1 || {
  echo "make install failed, see $LOG/install.log" >&2; tail -20 "$LOG/install.log" >&2; exit 1; }
say "done: $D/inst/bin/ocamlopt ; report tool: $BUILD/tools/bin/bisect-ppx-report"
