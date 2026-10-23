#include <cstdio>
#include <string>
#include "ada.h"
int main() {
  using P = ada::url_pattern_regex::std_regex_provider;
  ada::url_pattern_init init; init.search = "q='x'";
  auto p = ada::parse_url_pattern<P>(std::move(init));
  auto t = p->test(std::string_view("https://h/?q='x'"));
  printf("ada search component %s, test(https://h/?q='x') = %d\n", std::string(p->get_search()).c_str(), t && *t);
}
