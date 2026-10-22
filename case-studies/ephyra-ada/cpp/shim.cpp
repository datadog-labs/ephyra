// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

// Runs one case on ada::url or ada::url_aggregator and writes a transcript.
//
// A case, as bytes: the input, an optional base, then setter calls, every
// string as a little-endian u32 length and its bytes:
//   input | has_base: u8 | [base] | count: u32 | (setter: u8, value)*
// The transcript: can_parse: u8, then one step for the parse and one per
// setter, each
//   ok: u8 | [valid: u8, 11 components]   (components only when parsed)
// where a setter step's ok is the setter's result and valid is
// url_aggregator::validate() (always 1 for ada::url). The components are
// href, origin, protocol, username, password, host, hostname, port,
// pathname, search, hash.

#include <algorithm>
#include <csignal>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <type_traits>
#include <vector>

#include "ada.h"

#ifdef EA_SANITIZE
#include <fcntl.h>
#include <unistd.h>
#endif

namespace {

struct Reader {
  const uint8_t* at;
  const uint8_t* end;
  uint8_t byte() { return at < end ? *at++ : 0; }
  uint32_t u32() {
    uint32_t v = 0;
    for (int i = 0; i < 4; i++) v |= uint32_t(byte()) << (8 * i);
    return v;
  }
  std::string_view str() {
    size_t n = u32();
    if (size_t(end - at) < n) n = size_t(end - at);
    std::string_view s(reinterpret_cast<const char*>(at), n);
    at += n;
    return s;
  }
};

void put_str(std::string& out, std::string_view s) {
  uint32_t n = uint32_t(s.size());
  for (int i = 0; i < 4; i++) out.push_back(char((n >> (8 * i)) & 0xff));
  out.append(s);
}

template <class T>
bool valid(const T&) {
  return true;
}
template <>
bool valid(const ada::url_aggregator& u) {
  return u.validate();
}

template <class T>
void put_url(std::string& out, const T& u) {
  out.push_back(valid(u) ? 1 : 0);
  put_str(out, u.get_href());
  put_str(out, u.get_origin());
  put_str(out, u.get_protocol());
  put_str(out, u.get_username());
  put_str(out, u.get_password());
  put_str(out, u.get_host());
  put_str(out, u.get_hostname());
  put_str(out, u.get_port());
  put_str(out, u.get_pathname());
  put_str(out, u.get_search());
  put_str(out, u.get_hash());
}

template <class T>
bool set(T& u, uint8_t setter, std::string_view v) {
  switch (setter) {
    case 0: return u.set_href(v);
    case 1: return u.set_protocol(v);
    case 2: return u.set_username(v);
    case 3: return u.set_password(v);
    case 4: return u.set_host(v);
    case 5: return u.set_hostname(v);
    case 6: return u.set_port(v);
    case 7: return u.set_pathname(v);
    case 8: u.set_search(v); return true;
    case 9: u.set_hash(v); return true;
  }
  return false;
}

// The state predicates of a URL, for checking them against its components:
//   has_credentials | has_empty_hostname | has_hostname | has_port |
//   has_search | has_hash | has_non_empty_username | has_non_empty_password
//   (one byte each; the last two are 2 on ada::url, which lacks them) |
//   get_pathname_length: u32
template <class T>
void put_flags(std::string& out, const T& u) {
  for (bool b : {u.has_credentials(), u.has_empty_hostname(), u.has_hostname(), u.has_port(),
                 u.has_search(), u.has_hash()})
    out.push_back(b ? 1 : 0);
  if constexpr (std::is_same_v<T, ada::url_aggregator>) {
    out.push_back(u.has_non_empty_username() ? 1 : 0);
    out.push_back(u.has_non_empty_password() ? 1 : 0);
  } else {
    out.push_back(2);
    out.push_back(2);
  }
  uint32_t n = uint32_t(u.get_pathname_length());
  for (int i = 0; i < 4; i++) out.push_back(char((n >> (8 * i)) & 0xff));
}

// `put` writes a URL after each step: put_url for transcripts, put_flags for
// the predicates.
template <class T>
std::string run(Reader r, void (*put)(std::string&, const T&)) {
  std::string out;
  std::string_view input = r.str();
  std::optional<std::string_view> base;
  if (r.byte()) base = r.str();
  std::optional<T> base_url;
  bool base_ok = true;
  if (base) {
    auto b = ada::parse<T>(*base);
    base_ok = bool(b);
    if (b) base_url = std::move(*b);
  }
  out.push_back(base_ok && ada::can_parse(input, base ? &*base : nullptr) ? 1 : 0);
  if (!base_ok) {
    out.push_back(0);
    return out;
  }
  auto parsed = ada::parse<T>(input, base_url ? &*base_url : nullptr);
  out.push_back(parsed ? 1 : 0);
  if (!parsed) return out;
  T u = std::move(*parsed);
  put(out, u);
  uint32_t count = r.u32();
  for (uint32_t i = 0; i < count; i++) {
    uint8_t setter = r.byte();
    std::string_view v = r.str();
    out.push_back(set(u, setter, v) ? 1 : 0);
    put(out, u);
  }
  return out;
}

#ifdef EA_SANITIZE
// A sanitizer or libc++ hardening failure ends the process; the signal it
// raises (abort, trap) writes the case being run to $EA_CRASH_FILE, so it
// can be replayed. Everything here is async-signal-safe: the case lives in
// a static buffer and the handler only opens, writes and re-raises.
uint8_t current[1 << 20];
size_t current_len = 0;
int current_kind = 0;
uint32_t current_limit = 0;

void dump(int sig) {
  const char* path = getenv("EA_CRASH_FILE");
  if (path) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0) {
      uint8_t head[5] = {uint8_t(current_kind), uint8_t(current_limit), uint8_t(current_limit >> 8),
                         uint8_t(current_limit >> 16), uint8_t(current_limit >> 24)};
      (void)!write(fd, head, sizeof head);
      (void)!write(fd, current, current_len);
      close(fd);
    }
  }
  signal(sig, SIG_DFL);
  raise(sig);
}

struct Install {
  Install() {
    for (int sig : {SIGABRT, SIGTRAP, SIGILL, SIGSEGV, SIGBUS}) signal(sig, dump);
  }
} install;
#endif

void put_list(std::string& out, const std::vector<std::string>& items) {
  uint32_t n = uint32_t(items.size());
  for (int i = 0; i < 4; i++) out.push_back(char((n >> (8 * i)) & 0xff));
  for (auto& s : items) put_str(out, s);
}

// Runs a URLSearchParams case: the init string, then operations
//   count: u32 | (op: u8, key, value)*
// with ops 0 append, 1 delete, 2 delete(key, value), 3 set, 4 sort, 5 get,
// 6 getAll, 7 has, 8 has(key, value), 9 keys, 10 values, 11 entries (each
// a fresh iterator, run to the end), 12 next on one entries iterator made
// after the constructor. After the constructor and after each operation the
// transcript has: the operation's result (a list of strings: none, the
// value got, the values, "true" or "false", what the iterator yields, the
// next entry or none), size(), to_string(), and the entries as alternating
// keys and values.
std::string run_params(Reader r) {
  std::string out;
  ada::url_search_params p(r.str());
  auto step = [&](std::vector<std::string> result) {
    put_list(out, result);
    put_str(out, std::to_string(p.size()));
    put_str(out, p.to_string());
    std::vector<std::string> entries;
    for (auto& [k, v] : p) {
      entries.push_back(k);
      entries.push_back(v);
    }
    put_list(out, entries);
  };
  step({});
  // One live entries iterator for the whole case, as a script holding
  // `params.entries()` across the operations would.
  auto live = p.get_entries();
  auto collect = [](auto it) {
    std::vector<std::string> out;
    while (auto x = it.next()) out.emplace_back(*x);
    return out;
  };
  uint32_t count = r.u32();
  for (uint32_t i = 0; i < count; i++) {
    uint8_t op = r.byte();
    std::string_view k = r.str(), v = r.str();
    std::vector<std::string> result;
    switch (op) {
      case 0: p.append(k, v); break;
      case 1: p.remove(k); break;
      case 2: p.remove(k, v); break;
      case 3: p.set(k, v); break;
      case 4: p.sort(); break;
      case 5:
        if (auto got = p.get(k)) result.emplace_back(*got);
        break;
      case 6: result = p.get_all(k); break;
      case 7: result.emplace_back(p.has(k) ? "true" : "false"); break;
      case 8: result.emplace_back(p.has(k, v) ? "true" : "false"); break;
      case 9: result = collect(p.get_keys()); break;
      case 10: result = collect(p.get_values()); break;
      case 11: {
        auto it = p.get_entries();
        while (auto e = it.next()) {
          result.emplace_back(e->first);
          result.emplace_back(e->second);
        }
        break;
      }
      case 12:
        if (auto e = live.next()) {
          result.emplace_back(e->first);
          result.emplace_back(e->second);
        }
        break;
    }
    step(std::move(result));
  }
  return out;
}

// ada's std_regex_provider, except that a match the engine gives up on
// (libc++ throws error_complexity on nested quantifiers such as the spec's
// `((?:.*)*)` for `**`, past about a dozen input bytes) sets
// `regex_gave_up` instead of passing for "no match". run_pattern reports
// such an answer as 3, which comparisons skip: it is std::regex's limit,
// not ada's answer. The list and init runners use it too (one template
// instantiation for coverage) and ignore the flag.
thread_local bool regex_gave_up = false;
struct recording_regex_provider {
  using regex_type = std::regex;
  static std::optional<regex_type> create_instance(std::string_view pattern, bool ignore_case) {
    return ada::url_pattern_regex::std_regex_provider::create_instance(pattern, ignore_case);
  }
  static std::optional<std::vector<std::optional<std::string>>> regex_search(std::string_view input,
                                                                             const regex_type& pattern) {
    std::match_results<std::string_view::const_iterator> m;
    try {
      if (!std::regex_search(input.begin(), input.end(), m, pattern, std::regex_constants::match_any))
        return std::nullopt;
    } catch (const std::regex_error&) {
      regex_gave_up = true;
      return std::nullopt;
    }
    std::vector<std::optional<std::string>> groups;
    for (size_t i = 1; i < m.size(); ++i)
      groups.emplace_back(m[i].matched ? std::optional(m[i].str()) : std::nullopt);
    return groups;
  }
  static bool regex_match(std::string_view input, const regex_type& pattern) {
    try {
      return std::regex_match(input.begin(), input.end(), pattern);
    } catch (const std::regex_error&) {
      regex_gave_up = true;
      return false;
    }
  }
};

// Runs a URLPattern list case:
//   ignore_case: u8 | count: u32 | pattern* | count: u32 | pathname*
// The transcript: whether parse_url_pattern_list succeeded; then for each
// pattern whether it compiles alone (parse_url_pattern with only a
// pathname); then, if the list compiled, for each pathname:
//   ok: u8 (it parses as https://example.com<pathname>) | canonical
//   pathname | route: u32 (index + 1, 0 for none) | regexp: u8 |
//   truncated: u8 | values (list: captures, or regexp groups with a
//   missing group as "\x00") | then per route: matched alone: u8 | its
//   values ordered by the list's group_names(route).
std::string run_pattern_list(Reader r) {
  using provider = recording_regex_provider;
  std::string out;
  ada::url_pattern_options options{.ignore_case = r.byte() != 0};
  std::vector<std::string> patterns;
  for (uint32_t n = r.u32(), i = 0; i < n; i++) patterns.emplace_back(r.str());
  std::vector<std::string> paths;
  for (uint32_t n = r.u32(), i = 0; i < n; i++) paths.emplace_back(r.str());
  std::vector<std::string_view> views(patterns.begin(), patterns.end());
  auto list = ada::parse_url_pattern_list<provider>(std::span<const std::string_view>(views), nullptr,
                                                    &options);
  out.push_back(list ? 1 : 0);
  std::vector<std::optional<ada::url_pattern<provider>>> alone;
  for (auto& p : patterns) {
    ada::url_pattern_init init{};
    init.pathname = p;
    auto u = ada::parse_url_pattern<provider>(std::move(init), nullptr, &options);
    out.push_back(u ? 1 : 0);
    alone.emplace_back(u ? std::optional(std::move(*u)) : std::nullopt);
  }
  if (!list) return out;
  auto value = [](const std::optional<std::string>& v) { return v ? *v : std::string("\0", 1); };
  for (auto& path : paths) {
    auto url = ada::parse<ada::url_aggregator>("https://example.com" + path);
    out.push_back(url ? 1 : 0);
    if (!url) continue;
    std::string canonical(url->get_pathname());
    put_str(out, canonical);
    auto m = list->match(canonical);
    uint32_t route = m.has_match() ? uint32_t(m.route_index) + 1 : 0;
    for (int i = 0; i < 4; i++) out.push_back(char((route >> (8 * i)) & 0xff));
    out.push_back(m.regexp_route ? 1 : 0);
    out.push_back(m.captures_truncated ? 1 : 0);
    std::vector<std::string> values;
    if (m.regexp_route) {
      for (auto& g : m.regexp_groups) values.push_back(value(g));
    } else {
      for (uint32_t i = 0; i < m.capture_count; i++)
        values.push_back(canonical.substr(m.captures[i].offset, m.captures[i].length));
    }
    put_list(out, values);
    for (size_t k = 0; k < patterns.size(); k++) {
      std::vector<std::string> groups;
      bool matched = false;
      if (alone[k]) {
        ada::url_pattern_init input{};
        input.protocol = "https";
        input.hostname = "example.com";
        input.pathname = canonical;
        auto e = alone[k]->exec(ada::url_pattern_input(std::move(input)), nullptr);
        if (e && *e) {
          matched = true;
          auto& found = (**e).pathname.groups;
          for (auto& name : list->group_names(k)) {
            auto it = found.find(name);
            groups.push_back(it == found.end() ? std::string("\x01missing") : value(it->second));
          }
        }
      }
      out.push_back(matched ? 1 : 0);
      put_list(out, groups);
    }
  }
  return out;
}

// Runs a URLPattern case:
//   mode: u8 (0 constructor string, 1 init) | string, or 9 x (present: u8,
//   value) for protocol .. hash and baseURL | present: u8, base |
//   ignore_case: u8 | count: u32 | (input, present: u8, base)*
// where each input is a URL string. The transcript: ok: u8 (the pattern
// compiles); then the 8 component pattern strings and has_regexp_groups;
// then per input: test (0 false, 1 true, 2 error, 3 the regex engine gave
// up), exec (0 no match, 1 match, 2 error, 3 gave up), and for a match, per
// component its input and its groups
// sorted by name (count, then name and value, "\0" for undefined).
std::string run_pattern(Reader r) {
  using provider = recording_regex_provider;
  std::string out;
  uint8_t mode = r.byte();
  std::string text;
  ada::url_pattern_init init{};
  if (mode == 0) {
    text = std::string(r.str());
  } else {
    std::optional<std::string>* fields[] = {&init.protocol, &init.username, &init.password,
                                             &init.hostname, &init.port,     &init.pathname,
                                             &init.search,   &init.hash,     &init.base_url};
    for (auto* f : fields) {
      bool present = r.byte();
      std::string v(r.str());
      if (present) *f = v;
    }
  }
  bool has_base = r.byte();
  std::string base(r.str());
  ada::url_pattern_options options{.ignore_case = r.byte() != 0};
  std::string_view base_view(base);
  auto pattern =
      mode == 0 ? ada::parse_url_pattern<provider>(std::string_view(text), has_base ? &base_view : nullptr,
                                                   &options)
                : ada::parse_url_pattern<provider>(std::move(init), has_base ? &base_view : nullptr, &options);
  out.push_back(pattern ? 1 : 0);
  if (!pattern) return out;
  for (auto c : {pattern->get_protocol(), pattern->get_username(), pattern->get_password(),
                 pattern->get_hostname(), pattern->get_port(), pattern->get_pathname(),
                 pattern->get_search(), pattern->get_hash()})
    put_str(out, c);
  out.push_back(pattern->has_regexp_groups() ? 1 : 0);
  uint32_t count = r.u32();
  for (uint32_t i = 0; i < count; i++) {
    std::string input(r.str());
    bool input_has_base = r.byte();
    std::string input_base(r.str());
    std::string_view input_base_view(input_base);
    const std::string_view* b = input_has_base ? &input_base_view : nullptr;
    regex_gave_up = false;
    auto t = pattern->test(ada::url_pattern_input(std::string_view(input)), b);
    out.push_back(regex_gave_up ? 3 : t ? (*t ? 1 : 0) : 2);
    regex_gave_up = false;
    auto e = pattern->exec(ada::url_pattern_input(std::string_view(input)), b);
    bool gave_up = regex_gave_up;
    out.push_back(gave_up ? 3 : e ? (*e ? 1 : 0) : 2);
    if (gave_up || !e || !*e) continue;
    auto& m = **e;
    for (auto* c : {&m.protocol, &m.username, &m.password, &m.hostname, &m.port, &m.pathname,
                    &m.search, &m.hash}) {
      put_str(out, c->input);
      std::vector<std::pair<std::string, std::string>> groups;
      for (auto& [k, v] : c->groups) groups.emplace_back(k, v ? *v : std::string("\0", 1));
      std::sort(groups.begin(), groups.end());
      std::vector<std::string> flat;
      for (auto& [k, v] : groups) {
        flat.push_back(k);
        flat.push_back(v);
      }
      put_list(out, flat);
    }
  }
  return out;
}

// Runs URLPatternInit inputs through the all-wildcard pattern {}:
//   count: u32 | (9 x (present: u8, value))*   (protocol .. hash, baseURL)
// The transcript, per input: status (0 no match, 1 match, 2 error), then for
// a match the 8 component inputs exec reports (the processed init).
std::string run_init_inputs(Reader r) {
  using provider = recording_regex_provider;
  std::string out;
  auto pattern = ada::parse_url_pattern<provider>(ada::url_pattern_init{});
  uint32_t count = r.u32();
  for (uint32_t i = 0; i < count; i++) {
    ada::url_pattern_init init{};
    std::optional<std::string>* fields[] = {&init.protocol, &init.username, &init.password,
                                             &init.hostname, &init.port,     &init.pathname,
                                             &init.search,   &init.hash,     &init.base_url};
    for (auto* f : fields) {
      bool present = r.byte();
      std::string v(r.str());
      if (present) *f = v;
    }
    auto e = pattern->exec(ada::url_pattern_input(std::move(init)), nullptr);
    out.push_back(e ? (*e ? 1 : 0) : 2);
    if (!e || !*e) continue;
    auto& m = **e;
    for (auto* c : {&m.protocol, &m.username, &m.password, &m.hostname, &m.port, &m.pathname,
                    &m.search, &m.hash})
      put_str(out, c->input);
  }
  return out;
}

// Runs URLPatternInit inputs through test and exec of `{}`, alone and with
// a base URL string. Per input, four bytes, each 0 false or no match, 1 true
// or a match, 2 an error: test(init), exec(init), test(init, base),
// exec(init, base).
std::string run_init_test(Reader r) {
  using provider = recording_regex_provider;
  std::string out;
  auto pattern = ada::parse_url_pattern<provider>(ada::url_pattern_init{});
  uint32_t count = r.u32();
  for (uint32_t i = 0; i < count; i++) {
    ada::url_pattern_init init{};
    std::optional<std::string>* fields[] = {&init.protocol, &init.username, &init.password,
                                             &init.hostname, &init.port,     &init.pathname,
                                             &init.search,   &init.hash,     &init.base_url};
    for (auto* f : fields) {
      bool present = r.byte();
      std::string v(r.str());
      if (present) *f = v;
    }
    const std::string_view base = "https://example.com/";
    for (const std::string_view* b : {static_cast<const std::string_view*>(nullptr), &base}) {
      auto t = pattern->test(ada::url_pattern_input(init), b);
      out.push_back(t ? (*t ? 1 : 0) : 2);
      auto e = pattern->exec(ada::url_pattern_input(init), b);
      out.push_back(e ? (*e ? 1 : 0) : 2);
    }
  }
  return out;
}

// Runs domains through ada's IDNA directly:  count: u32 | domain*
// Per domain: to_ascii ok: u8, to_ascii out, to_unicode out. to_ascii uses
// the bool overload (false on error, no empty-string ambiguity); to_unicode
// is best-effort and never signals an error.
std::string run_idna(Reader r) {
  std::string out;
  uint32_t count = r.u32();
  for (uint32_t i = 0; i < count; i++) {
    std::string_view domain = r.str();
    std::string ascii;
    bool ok = ada::idna::to_ascii(domain, ascii);
    out.push_back(ok ? 1 : 0);
    put_str(out, ascii);
    put_str(out, ada::idna::to_unicode(domain));
  }
  return out;
}

}  // namespace

extern "C" {

// Runs domains through ada's IDNA (see run_idna); the transcript is
// malloc'd, freed with ea_free.
size_t ea_idna(const uint8_t* bytes, size_t len, char** out) {
#ifdef EA_SANITIZE
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  current_kind = 6;
#endif
  std::string t = run_idna(Reader{bytes, bytes + len});
  *out = static_cast<char*>(std::malloc(t.size() ? t.size() : 1));
  std::memcpy(*out, t.data(), t.size());
  return t.size();
}

// Runs URLPatternInit inputs (see run_init_inputs); the transcript is
// malloc'd, freed with ea_free.
size_t ea_init_inputs(const uint8_t* bytes, size_t len, char** out) {
#ifdef EA_SANITIZE
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  current_kind = 5;
#endif
  std::string t = run_init_inputs(Reader{bytes, bytes + len});
  *out = static_cast<char*>(std::malloc(t.size() ? t.size() : 1));
  std::memcpy(*out, t.data(), t.size());
  return t.size();
}

// Runs URLPatternInit inputs through test and exec (see run_init_test);
// the transcript is malloc'd, freed with ea_free.
size_t ea_init_test(const uint8_t* bytes, size_t len, char** out) {
#ifdef EA_SANITIZE
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  current_kind = 5;
#endif
  std::string t = run_init_test(Reader{bytes, bytes + len});
  *out = static_cast<char*>(std::malloc(t.size() ? t.size() : 1));
  std::memcpy(*out, t.data(), t.size());
  return t.size();
}

// Runs a URLPattern case (see run_pattern); the transcript is malloc'd,
// freed with ea_free.
size_t ea_pattern(const uint8_t* bytes, size_t len, char** out) {
#ifdef EA_SANITIZE
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  current_kind = 4;
#endif
  std::string t = run_pattern(Reader{bytes, bytes + len});
  *out = static_cast<char*>(std::malloc(t.size() ? t.size() : 1));
  std::memcpy(*out, t.data(), t.size());
  return t.size();
}

// Runs a URLPattern list case (see run_pattern_list); the transcript is
// malloc'd, freed with ea_free.
size_t ea_pattern_list(const uint8_t* bytes, size_t len, char** out) {
#ifdef EA_SANITIZE
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  current_kind = 3;
#endif
  std::string t = run_pattern_list(Reader{bytes, bytes + len});
  *out = static_cast<char*>(std::malloc(t.size() ? t.size() : 1));
  std::memcpy(*out, t.data(), t.size());
  return t.size();
}

// Runs a URLSearchParams case (see run_params); the transcript is malloc'd,
// freed with ea_free.
size_t ea_params(const uint8_t* bytes, size_t len, char** out) {
#ifdef EA_SANITIZE
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  current_kind = 2;
#endif
  std::string t = run_params(Reader{bytes, bytes + len});
  *out = static_cast<char*>(std::malloc(t.size() ? t.size() : 1));
  std::memcpy(*out, t.data(), t.size());
  return t.size();
}

#ifdef EA_SANITIZE
// Reads one byte past a heap buffer, while `bytes` stands as the current
// case: the sanitized build's canary.
int ea_canary(const uint8_t* bytes, size_t len) {
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  char* p = static_cast<char*>(std::malloc(8));
  std::memset(p, 0, 8);
  volatile char c = p[8];
  std::free(p);
  return c;
}

const char* __asan_default_options() { return "abort_on_error=1:detect_leaks=0:handle_abort=0"; }
const char* __ubsan_default_options() {
  return "halt_on_error=1:abort_on_error=1:print_stacktrace=1";
}
#endif

// kind 0 is ada::url, 1 is ada::url_aggregator; 2 and 3 the same with
// put_flags instead of put_url. `max_input_length` 0 keeps
// ada's default. The transcript is malloc'd; free it with ea_free.
size_t ea_run(int kind, const uint8_t* bytes, size_t len, uint32_t max_input_length,
              char** out) {
#ifdef EA_SANITIZE
  current_len = len < sizeof current ? len : sizeof current;
  std::memcpy(current, bytes, current_len);
  current_kind = kind & 1;  // flags runs replay as the transcript of the same representation
  current_limit = max_input_length;
#endif
  uint32_t before = ada::get_max_input_length();
  if (max_input_length) ada::set_max_input_length(max_input_length);
  Reader r{bytes, bytes + len};
  std::string t = kind == 0   ? run<ada::url>(r, put_url<ada::url>)
                  : kind == 1 ? run<ada::url_aggregator>(r, put_url<ada::url_aggregator>)
                  : kind == 2 ? run<ada::url>(r, put_flags<ada::url>)
                              : run<ada::url_aggregator>(r, put_flags<ada::url_aggregator>);
  ada::set_max_input_length(before);
  *out = static_cast<char*>(std::malloc(t.size() ? t.size() : 1));
  std::memcpy(*out, t.data(), t.size());
  return t.size();
}

void ea_free(char* p) { std::free(p); }

}
