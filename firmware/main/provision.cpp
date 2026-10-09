#include "provision.hpp"

#include <cstring>

namespace quota::device {
void clear_sensitive(void* memory, size_t size) {
    volatile unsigned char* bytes = static_cast<volatile unsigned char*>(memory);
    while (size--) *bytes++ = 0;
}

std::optional<uint32_t> mapped_ipv4(const uint8_t (&address)[16]) {
    for (size_t i = 0; i < 10; ++i) if (address[i] != 0) return std::nullopt;
    if (address[10] != 0xff || address[11] != 0xff) return std::nullopt;
    return (uint32_t{address[12]} << 24) | (uint32_t{address[13]} << 16) |
           (uint32_t{address[14]} << 8) | uint32_t{address[15]};
}

namespace {
int hex(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

bool decode(std::string_view input, char* output, size_t capacity, bool pem) {
    size_t length = 0;
    for (size_t i = 0; i < input.size(); ++i) {
        unsigned char c = input[i];
        if (c == '%') {
            if (i + 2 >= input.size()) return false;
            const int high = hex(input[i + 1]), low = hex(input[i + 2]);
            if (high < 0 || low < 0) return false;
            c = static_cast<unsigned char>((high << 4) | low);
            i += 2;
        } else if (c == '+') {
            c = ' ';
        }
        if (c == 0 || c == 127 || (c < 32 && !(pem && (c == '\r' || c == '\n')))) return false;
        if (length + 1 >= capacity) return false;
        output[length++] = static_cast<char>(c);
    }
    output[length] = 0;
    return true;
}

bool valid(Credentials& result, const char* port, const char* tls) {
    if (!result.ssid[0] || !result.mqtt_host[0] || !result.mqtt_username[0] ||
        !result.mqtt_password[0] || std::strlen(result.token) != 32) return false;
    for (const char* p = result.token; *p; ++p) if (hex(*p) < 0) return false;
    const size_t password_length = std::strlen(result.wifi_password);
    if (password_length && password_length < 8) return false;
    if (password_length == 64) {
        for (const char* p = result.wifi_password; *p; ++p) if (hex(*p) < 0) return false;
    }
    for (const char* p = result.mqtt_host; *p; ++p) {
        const char c = *p;
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
              (c >= '0' && c <= '9') || c == '-' || c == '.' || c == ':')) return false;
    }
    uint32_t number = 0;
    if (!*port) return false;
    for (const char* p = port; *p; ++p) {
        if (*p < '0' || *p > '9') return false;
        number = number * 10 + static_cast<unsigned>(*p - '0');
        if (number > 65535) return false;
    }
    if (number == 0) return false;
    result.mqtt_port = static_cast<uint16_t>(number);
    if (std::strcmp(tls, "0") != 0 && std::strcmp(tls, "1") != 0) return false;
    result.tls = tls[0] == '1';
    const std::string_view ca(result.ca);
    if (result.tls) {
        if (ca.find("-----BEGIN CERTIFICATE-----") != 0 ||
            ca.find("-----END CERTIFICATE-----") == std::string_view::npos) return false;
    } else if (!ca.empty()) {
        return false;
    }
    return true;
}
} // namespace

bool parse_provisioning(std::string_view body, Credentials& result) {
    clear_sensitive(&result, sizeof(result));
    if (body.empty() || body.size() > kMaxFormBytes) return false;
    char port[6]{}, tls[2]{};
    struct Field { std::string_view name; char* output; size_t capacity; };
    const Field fields[] = {
        {"ssid", result.ssid, sizeof(result.ssid)},
        {"wifi_password", result.wifi_password, sizeof(result.wifi_password)},
        {"mqtt_host", result.mqtt_host, sizeof(result.mqtt_host)},
        {"mqtt_port", port, sizeof(port)},
        {"mqtt_username", result.mqtt_username, sizeof(result.mqtt_username)},
        {"mqtt_password", result.mqtt_password, sizeof(result.mqtt_password)},
        {"tls", tls, sizeof(tls)}, {"ca", result.ca, sizeof(result.ca)},
        {"token", result.token, sizeof(result.token)}
    };
    unsigned seen = 0;
    bool accepted = true;
    while (!body.empty() && accepted) {
        const size_t end = body.find('&');
        const auto pair = body.substr(0, end);
        const size_t equals = pair.find('=');
        if (equals == std::string_view::npos || equals == 0) { accepted = false; break; }
        const auto name = pair.substr(0, equals);
        size_t index = 0;
        while (index < sizeof(fields) / sizeof(fields[0]) && fields[index].name != name) ++index;
        if (index == sizeof(fields) / sizeof(fields[0]) || (seen & (1U << index))) {
            accepted = false;
            break;
        }
        seen |= 1U << index;
        accepted = decode(pair.substr(equals + 1), fields[index].output, fields[index].capacity, name == "ca");
        if (end == std::string_view::npos) break;
        body.remove_prefix(end + 1);
        if (body.empty()) accepted = false;
    }
    accepted = accepted && seen == 511 && valid(result, port, tls);
    clear_sensitive(port, sizeof(port));
    clear_sensitive(tls, sizeof(tls));
    if (!accepted) clear_sensitive(&result, sizeof(result));
    return accepted;
}

bool abandon_unsaved_setup(bool persisted, int failed_attempts) {
    return !persisted && failed_attempts >= kUnsavedJoinAttempts;
}
} // namespace quota::device

#ifdef QUOTA_PROVISION_SELF_TEST
#include <cassert>
#include <initializer_list>
#include <string>
int main() {
    quota::device::Credentials result;
    const std::string form = "ssid=Test+WiFi&wifi_password=synthetic123&mqtt_host=broker.local&mqtt_port=1883&"
        "mqtt_username=device&mqtt_password=synthetic%26password&tls=0&ca=&"
        "token=0123456789abcdef0123456789abcdef";
    assert(quota::device::parse_provisioning(form, result));
    assert(std::strcmp(result.ssid, "Test WiFi") == 0 && std::strcmp(result.mqtt_password, "synthetic&password") == 0);
    assert(result.mqtt_port == 1883 && !result.tls);
    for (const std::string& invalid : {
        form + "&ssid=duplicate", form + "&unknown=value", form + "&", std::string("ssid=bad%00"),
        form.substr(0, form.find("token=")),
        std::string("ssid=") + std::string(33, 'x') + form.substr(form.find('&')),
        form.substr(0, form.find("1883")) + "65536" + form.substr(form.find("1883") + 4),
        form.substr(0, form.find("synthetic%26password")) + "%GG" + form.substr(form.find("&tls=")),
        form.substr(0, form.find("tls=0")) + "tls=1&ca=&token=0123456789abcdef0123456789abcdef",
        // The timezone field was removed from the form: a stale client must be rejected.
        form + "&timezone=ICT-7"
    }) {
        assert(!quota::device::parse_provisioning(invalid, result));
        assert(!result.ssid[0] && !result.mqtt_password[0]);
    }
    quota::device::clear_sensitive(&result, sizeof(result));
    // ESP-IDF's HTTP server listens on an IPv6 socket when lwIP IPv6 is enabled,
    // so the setup client's IPv4 address arrives as ::ffff:192.168.4.2.
    const uint8_t mapped[16] = {0,0,0,0,0,0,0,0,0,0,0xff,0xff,192,168,4,2};
    assert(quota::device::mapped_ipv4(mapped) == std::optional<uint32_t>(0xC0A80402U));
    const uint8_t native_v6[16] = {0xfe,0x80,0,0,0,0,0,0,0,0,0xff,0xff,192,168,4,2};
    assert(!quota::device::mapped_ipv4(native_v6));
    const uint8_t compatible[16] = {0,0,0,0,0,0,0,0,0,0,0,0,192,168,4,2};
    assert(!quota::device::mapped_ipv4(compatible));
    assert(!quota::device::abandon_unsaved_setup(false, 0) && !quota::device::abandon_unsaved_setup(false, 2));
    assert(quota::device::abandon_unsaved_setup(false, 3) && quota::device::abandon_unsaved_setup(false, 6));
    assert(!quota::device::abandon_unsaved_setup(true, 0) && !quota::device::abandon_unsaved_setup(true, 6) && !quota::device::abandon_unsaved_setup(true, 100));
}
#endif
