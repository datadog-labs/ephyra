// A protocol pattern of an escaped colon makes canonicalize_protocol index
// an empty string_view. Without library assertions this prints "error" three
// times; with libc++ hardening it aborts on the first pattern:
//
//   clang++ -std=c++20 -D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_FAST \
//     -DADA_INCLUDE_URL_PATTERN=1 -DADA_USE_UNSAFE_STD_REGEX_PROVIDER \
//     -I<ada singleheader> <ada singleheader>/ada.cpp repro.cpp
#include <cstdio>
#include "ada.h"
int main() {
  using P = ada::url_pattern_regex::std_regex_provider;
  for (const char* src : {"\\:", "\\:?", "\\:*"}) {
    ada::url_pattern_init init; init.protocol = src;
    auto p = ada::parse_url_pattern<P>(std::move(init));
    printf("protocol %-4s -> %s\n", src, p ? "pattern" : "error");
  }
}
