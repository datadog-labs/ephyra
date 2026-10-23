#!/usr/bin/env python3
# Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
# This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

"""Patch bisect_ppx's instrumenter to record absolute source paths.

dune runs the ppx from _build/default with relative input names, and files
copied by (copy_files#) are recorded under their copy's basename. Consumers
of the .coverage files (bisect-ppx-report cobertura run from another cwd)
then cannot find the sources. With BISECT_SOURCE_ROOT set at build time the
recorded name becomes <root>/<path>, where <path> is the name from the
file's first location (it follows the `# 1 "dir/file.ml"` line directive
of copied files) when that file exists under the root, else the input name.

usage: bisect-abs-path.py <bisect_ppx-src-dir>/src/ppx/instrument.ml"""
import sys
p = sys.argv[1]
s = open(p).read()
old = """          let runtime_initialization =
            Generated_code.runtime_initialization points path in"""
new = """          let path =
            match Sys.getenv_opt "BISECT_SOURCE_ROOT" with
            | Some root when root <> "" ->
              let from_loc =
                match ast with
                | item :: _ ->
                  item.Parsetree.pstr_loc.Location.loc_start.Lexing.pos_fname
                | [] -> path
              in
              let abs q =
                if Filename.is_relative q then Filename.concat root q else q in
              if from_loc <> "" && Sys.file_exists (abs from_loc)
              then abs from_loc
              else abs path
            | _ -> path
          in
          let runtime_initialization =
            Generated_code.runtime_initialization points path in"""
if new in s:
    sys.exit(0)
assert old in s, "bisect_ppx instrument.ml layout changed"
open(p, "w").write(s.replace(old, new))
