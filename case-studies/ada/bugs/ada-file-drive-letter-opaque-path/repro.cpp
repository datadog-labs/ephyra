// new URL("C|", "file:") is marked as having an opaque path, though its path
// is the list ["C:"]: its host and path setters refuse to run, and it cannot
// serve as a base. The same href, parsed afresh, behaves.
//
// clang++ -std=c++20 -I<ada singleheader> <ada singleheader>/ada.cpp repro.cpp
#include <cstdio>
#include <string>

#include "ada.h"

template <class T>
void check(const char* name) {
  auto file = ada::parse<T>("file:");
  auto url = ada::parse<T>("C|", &*file);
  auto same = ada::parse<T>(url->get_href());
  for (auto* u : {&*url, &*same}) {
    T host = *u, path = *u;
    bool host_ok = host.set_hostname("h");
    bool path_ok = path.set_pathname("/D:/y");
    auto rel = ada::parse<T>("x", u);
    printf("%-10s %-24s has_opaque_path %d | set_hostname(\"h\") %d -> %-12s | "
           "set_pathname(\"/D:/y\") %d -> %-13s | new URL(\"x\", it) %s\n",
           name, u == &*url ? "new URL(\"C|\", \"file:\")" : "new URL(\"file:///C:\")",
           u->has_opaque_path, host_ok, std::string(host.get_href()).c_str(), path_ok,
           std::string(path.get_href()).c_str(),
           rel ? std::string(rel->get_href()).c_str() : "failure");
  }
}

int main() {
  check<ada::url>("url");
  check<ada::url_aggregator>("aggregator");
}
