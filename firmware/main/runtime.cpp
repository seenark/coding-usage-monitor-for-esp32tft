#include "board.hpp"
#include "provision.hpp"
#include "quota_model.hpp"
#include "quota_ui.hpp"

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <new>
#include <string_view>
#include <sys/socket.h>
#include <time.h>

#include "esp_event.h"
#include "esp_http_server.h"
#include "esp_log.h"
#include "esp_netif.h"
#include "esp_random.h"
#include "esp_sntp.h"
#include "esp_timer.h"
#include "esp_wifi.h"
#include "freertos/FreeRTOS.h"
#include "freertos/semphr.h"
#include "freertos/task.h"
#include "lwip/sockets.h"
#include "esp_mac.h"
#include "mbedtls/x509_crt.h"
#include "mqtt_client.h"
#include "nvs.h"
#include "nvs_flash.h"
#include "sdkconfig.h"

namespace {
using quota::device::Credentials;
using quota::device::clear_sensitive;
constexpr size_t kMaxFrame = 16384;
constexpr const char* kTopics[] = {"ai-quota/v1/providers/claude", "ai-quota/v1/providers/codex", "ai-quota/v1/server/status", quota::device_config_topic};
constexpr const char* kStatusTopic = "ai-quota/v1/device/status";
constexpr char kStatusOnline[] = "{\"state\":\"online\"}";
constexpr char kStatusOffline[] = "{\"state\":\"offline\"}";
constexpr const char* kSetupAddress = "http://192.168.4.1";
enum class Stage { Setup, Connecting, Online };
quota::Model model;
quota::Clock clock_model;
quota::Ui ui(model, clock_model);
SemaphoreHandle_t ui_mutex = nullptr;
Credentials credentials;
std::atomic<Stage> stage{Stage::Setup};
std::atomic<bool> provision_requested{false};
std::atomic<bool> station_online{false};
std::atomic<bool> station_lost{false};
std::atomic<bool> trusted_clock{false};
httpd_handle_t http_server = nullptr;
esp_mqtt_client_handle_t mqtt = nullptr;
char ap_ssid[33]{};
char ap_password[9]{};
char csrf_token[33]{};
char client_id[32]{};
char frame_topic[64]{};
char frame[kMaxFrame];
size_t frame_size = 0, frame_received = 0;
bool frame_active = false;

int64_t milliseconds() { return esp_timer_get_time() / 1000; }

// One lock protects LVGL, the shared model and clock. Network operations never hold it.
class UiLock {
public:
    UiLock() { xSemaphoreTake(ui_mutex, portMAX_DELAY); }
    ~UiLock() { xSemaphoreGive(ui_mutex); }
    UiLock(const UiLock&) = delete;
    UiLock& operator=(const UiLock&) = delete;
};

// Device setup and last-known display config live in NVS namespace "quota", unencrypted (ADR 0002).
// Values are never logged.
// Values from the setup form are RAM-only until the station gets an IP; boot-loaded values are already stored.
bool credentials_persisted = false;
constexpr const char* kNvsNamespace = "quota";
constexpr const char* kCredentialKeys[] = {"ssid", "wifi_pass", "mqtt_host", "mqtt_port", "mqtt_user", "mqtt_pass", "mqtt_tls", "mqtt_ca"};
quota::DeviceConfig applied_config;

class Nvs {
public:
    explicit Nvs(nvs_open_mode_t mode) { open_ = nvs_open(kNvsNamespace, mode, &handle_) == ESP_OK; }
    ~Nvs() { if (open_) nvs_close(handle_); }
    Nvs(const Nvs&) = delete;
    Nvs& operator=(const Nvs&) = delete;
    bool ok() const { return open_; }
    nvs_handle_t handle() const { return handle_; }
private:
    nvs_handle_t handle_ = 0;
    bool open_ = false;
};

void init_nvs() {
    esp_err_t error = nvs_flash_init();
    if (error == ESP_ERR_NVS_NO_FREE_PAGES || error == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        error = nvs_flash_init();
    }
    ESP_ERROR_CHECK(error);
}

bool read_string(nvs_handle_t handle, const char* key, char* output, size_t capacity) {
    size_t size = capacity;
    return nvs_get_str(handle, key, output, &size) == ESP_OK;
}

// Complete Device setup only: SSID, broker host, user and password are required (the Wi-Fi password may be empty).
bool load_credentials(Credentials& result) {
    clear_sensitive(&result, sizeof(result));
    Nvs nvs(NVS_READONLY);
    if (!nvs.ok()) return false;
    const auto handle = nvs.handle();
    uint8_t tls = 0;
    size_t ca_size = sizeof(result.ca) - 1;
    bool complete = read_string(handle, "ssid", result.ssid, sizeof(result.ssid)) &&
        read_string(handle, "mqtt_host", result.mqtt_host, sizeof(result.mqtt_host)) &&
        read_string(handle, "mqtt_user", result.mqtt_username, sizeof(result.mqtt_username)) &&
        read_string(handle, "mqtt_pass", result.mqtt_password, sizeof(result.mqtt_password)) &&
        nvs_get_u16(handle, "mqtt_port", &result.mqtt_port) == ESP_OK &&
        nvs_get_u8(handle, "mqtt_tls", &tls) == ESP_OK && tls <= 1;
    read_string(handle, "wifi_pass", result.wifi_password, sizeof(result.wifi_password));
    result.tls = tls == 1;
    if (complete && result.tls) complete = nvs_get_blob(handle, "mqtt_ca", result.ca, &ca_size) == ESP_OK && ca_size > 0;
    complete = complete && result.ssid[0] && result.mqtt_host[0] && result.mqtt_username[0] && result.mqtt_password[0] && result.mqtt_port != 0;
    if (!complete) clear_sensitive(&result, sizeof(result));
    return complete;
}

bool save_credentials(const Credentials& value) {
    Nvs nvs(NVS_READWRITE);
    if (!nvs.ok()) return false;
    const auto handle = nvs.handle();
    bool saved = nvs_set_str(handle, "ssid", value.ssid) == ESP_OK &&
        nvs_set_str(handle, "wifi_pass", value.wifi_password) == ESP_OK &&
        nvs_set_str(handle, "mqtt_host", value.mqtt_host) == ESP_OK &&
        nvs_set_u16(handle, "mqtt_port", value.mqtt_port) == ESP_OK &&
        nvs_set_str(handle, "mqtt_user", value.mqtt_username) == ESP_OK &&
        nvs_set_str(handle, "mqtt_pass", value.mqtt_password) == ESP_OK &&
        nvs_set_u8(handle, "mqtt_tls", value.tls ? 1 : 0) == ESP_OK;
    if (saved) {
        if (value.tls) saved = nvs_set_blob(handle, "mqtt_ca", value.ca, std::strlen(value.ca) + 1) == ESP_OK;
        else nvs_erase_key(handle, "mqtt_ca");
    }
    return saved && nvs_commit(handle) == ESP_OK;
}

// Erases Device setup only; the last-known layout, timezone and brightness stay.
void erase_stored_credentials() {
    Nvs nvs(NVS_READWRITE);
    if (!nvs.ok()) return;
    for (const char* key : kCredentialKeys) nvs_erase_key(nvs.handle(), key);
    nvs_commit(nvs.handle());
}

// Each stored field is validated again; anything missing or bad keeps the compiled-in default.
void load_config(quota::DeviceConfig& config) {
    Nvs nvs(NVS_READONLY);
    if (!nvs.ok()) return;
    const auto handle = nvs.handle();
    quota::Text timezone{};
    uint8_t layout = 0, brightness = 0;
    if (read_string(handle, "timezone", timezone.data(), timezone.size()) && quota::valid_timezone(timezone.data())) config.timezone = timezone;
    if (nvs_get_u8(handle, "layout", &layout) == ESP_OK && layout <= static_cast<uint8_t>(quota::Layout::Terminal)) config.layout = static_cast<quota::Layout>(layout);
    if (nvs_get_u8(handle, "brightness", &brightness) == ESP_OK && brightness >= 10 && brightness <= 100) config.brightness = brightness;
}

// nvs_set_* skips the flash write when the stored value is already identical.
void save_config(const quota::DeviceConfig& config) {
    Nvs nvs(NVS_READWRITE);
    if (!nvs.ok()) return;
    const auto handle = nvs.handle();
    if (nvs_set_str(handle, "timezone", config.timezone.data()) == ESP_OK &&
        nvs_set_u8(handle, "layout", static_cast<uint8_t>(config.layout)) == ESP_OK &&
        nvs_set_u8(handle, "brightness", static_cast<uint8_t>(config.brightness)) == ESP_OK) nvs_commit(handle);
}

bool allowed_topic(std::string_view topic) {
    for (const char* known : kTopics) if (topic == known) return true;
    return false;
}

void mqtt_event(void*, esp_event_base_t, int32_t event_id, void* event_data) {
    const auto event = static_cast<esp_mqtt_event_handle_t>(event_data);
    if (event_id == MQTT_EVENT_CONNECTED) {
        frame_active = false;
        bool subscribed = true;
        for (const char* topic : kTopics) {
            if (esp_mqtt_client_subscribe(event->client, topic, 1) < 0) subscribed = false;
        }
        // The device writes only this one topic; the broker ACL enforces that.
        esp_mqtt_client_publish(event->client, kStatusTopic, kStatusOnline, sizeof(kStatusOnline) - 1, 1, 1);
        UiLock lock;
        model.mqtt_connected = subscribed;
    } else if (event_id == MQTT_EVENT_DISCONNECTED || event_id == MQTT_EVENT_ERROR) {
        frame_active = false;
        UiLock lock;
        model.mqtt_connected = false;
    } else if (event_id == MQTT_EVENT_DATA) {
        if (event->current_data_offset == 0) {
            frame_active = false;
            if (event->total_data_len <= 0 || event->total_data_len > static_cast<int>(kMaxFrame) ||
                event->topic_len <= 0 || event->topic_len >= static_cast<int>(sizeof(frame_topic)) || !event->topic) return;
            const std::string_view topic(event->topic, event->topic_len);
            if (!allowed_topic(topic)) return;
            std::memcpy(frame_topic, topic.data(), topic.size());
            frame_topic[topic.size()] = 0;
            frame_size = event->total_data_len;
            frame_received = 0;
            frame_active = true;
        }
        if (!frame_active || event->current_data_offset < 0 || event->data_len <= 0 || !event->data ||
            event->total_data_len != static_cast<int>(frame_size) ||
            static_cast<size_t>(event->current_data_offset) != frame_received ||
            static_cast<size_t>(event->data_len) > frame_size - frame_received) {
            frame_active = false;
            return;
        }
        std::memcpy(frame + frame_received, event->data, event->data_len);
        frame_received += event->data_len;
        if (frame_received == frame_size) {
            frame_active = false;
            UiLock lock;
            model.ingest(std::string_view(frame_topic), std::string_view(frame, frame_size));
        }
    }
}

void start_mqtt() {
    if (mqtt || !station_online.load() || (credentials.tls && !trusted_clock.load())) return;
    esp_mqtt_client_config_t configuration{};
    configuration.broker.address.hostname = credentials.mqtt_host;
    configuration.broker.address.port = credentials.mqtt_port;
    configuration.broker.address.transport = credentials.tls ? MQTT_TRANSPORT_OVER_SSL : MQTT_TRANSPORT_OVER_TCP;
    configuration.broker.verification.certificate = credentials.tls ? credentials.ca : nullptr;
    configuration.broker.verification.skip_cert_common_name_check = false;
    configuration.credentials.username = credentials.mqtt_username;
    configuration.credentials.client_id = client_id;
    configuration.credentials.authentication.password = credentials.mqtt_password;
    configuration.session.protocol_ver = MQTT_PROTOCOL_V_3_1_1;
    configuration.session.keepalive = 30;
    // Retained "offline" appears if the device drops without a clean disconnect.
    configuration.session.last_will.topic = kStatusTopic;
    configuration.session.last_will.msg = kStatusOffline;
    configuration.session.last_will.msg_len = sizeof(kStatusOffline) - 1;
    configuration.session.last_will.qos = 1;
    configuration.session.last_will.retain = true;
    configuration.network.reconnect_timeout_ms = 5000;
    configuration.network.timeout_ms = 10000;
    configuration.buffer.size = 2048;
    configuration.buffer.out_size = 512;
    configuration.outbox.limit = 2048;
    configuration.task.stack_size = 8192;
    mqtt = esp_mqtt_client_init(&configuration);
    if (!mqtt) return;
    if (esp_mqtt_client_register_event(mqtt, MQTT_EVENT_ANY, mqtt_event, nullptr) != ESP_OK ||
        esp_mqtt_client_start(mqtt) != ESP_OK) {
        esp_mqtt_client_destroy(mqtt);
        mqtt = nullptr;
    }
}

void time_sync(struct timeval* time) {
    if (!time || time->tv_sec <= 0) return;
    UiLock lock;
    clock_model.sync(static_cast<int64_t>(time->tv_sec), milliseconds());
    trusted_clock.store(true);
}

void wifi_event(void*, esp_event_base_t base, int32_t event_id, void*) {
    if (base == IP_EVENT && event_id == IP_EVENT_STA_GOT_IP) {
        station_online.store(true);
        station_lost.store(false);
    } else if (base == WIFI_EVENT && event_id == WIFI_EVENT_STA_DISCONNECTED) {
        station_online.store(false);
        station_lost.store(true);
        UiLock lock;
        model.mqtt_connected = false;
    }
}

// ESP-IDF's HTTP server uses an IPv6 socket when lwIP IPv6 is enabled, so IPv4
// clients appear as IPv4-mapped addresses. Accept both forms, nothing else.
std::optional<uint32_t> socket_ipv4(const sockaddr_storage& address) {
    if (address.ss_family == AF_INET)
        return ntohl(reinterpret_cast<const sockaddr_in&>(address).sin_addr.s_addr);
    if (address.ss_family == AF_INET6) {
        uint8_t bytes[16];
        std::memcpy(bytes, &reinterpret_cast<const sockaddr_in6&>(address).sin6_addr, sizeof(bytes));
        return quota::device::mapped_ipv4(bytes);
    }
    return std::nullopt;
}

bool from_setup_interface(httpd_req_t* request) {
    constexpr uint32_t kSetupHost = 0xC0A80401U;  // 192.168.4.1
    constexpr uint32_t kSetupSubnet = 0xC0A80400U; // 192.168.4.0/24
    sockaddr_storage local{}, peer{};
    socklen_t size = sizeof(local);
    const int socket = httpd_req_to_sockfd(request);
    if (getsockname(socket, reinterpret_cast<sockaddr*>(&local), &size) != 0 ||
        socket_ipv4(local) != std::optional<uint32_t>(kSetupHost)) return false;
    size = sizeof(peer);
    if (getpeername(socket, reinterpret_cast<sockaddr*>(&peer), &size) != 0) return false;
    const auto peer_ipv4 = socket_ipv4(peer);
    if (!peer_ipv4 || (*peer_ipv4 & 0xffffff00U) != kSetupSubnet) return false;
    // Reject DNS rebinding and cross-origin browser requests before reading secrets.
    char host[32]{}, origin[64]{};
    if (httpd_req_get_hdr_value_str(request, "Host", host, sizeof(host)) != ESP_OK ||
        (std::strcmp(host, "192.168.4.1") != 0 && std::strcmp(host, "192.168.4.1:80") != 0)) return false;
    if (httpd_req_get_hdr_value_len(request, "Origin") != 0 &&
        (httpd_req_get_hdr_value_str(request, "Origin", origin, sizeof(origin)) != ESP_OK ||
         (std::strcmp(origin, kSetupAddress) != 0 && std::strcmp(origin, "http://192.168.4.1:80") != 0))) return false;
    return true;
}

esp_err_t respond(httpd_req_t* request, const char* status, const char* text) {
    httpd_resp_set_status(request, status);
    httpd_resp_set_type(request, "text/plain; charset=utf-8");
    httpd_resp_set_hdr(request, "Cache-Control", "no-store");
    httpd_resp_set_hdr(request, "Connection", "close");
    return httpd_resp_send(request, text, HTTPD_RESP_USE_STRLEN);
}

esp_err_t setup_form(httpd_req_t* request) {
    if (!from_setup_interface(request)) return respond(request, "403 Forbidden", "Setup is available only on the temporary AP.");
    if (stage.load() != Stage::Setup) return respond(request, "409 Conflict", "Connection in progress. Check the display.");
    httpd_resp_set_type(request, "text/html; charset=utf-8");
    httpd_resp_set_hdr(request, "Cache-Control", "no-store");
    httpd_resp_set_hdr(request, "Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'");
    httpd_resp_set_hdr(request, "X-Content-Type-Options", "nosniff");
    static constexpr char prefix[] =
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'>"
        "<title>AI Quota Monitor setup</title><style>body{font:16px sans-serif;max-width:32em;margin:2em auto;padding:1em}"
        "label{display:block;margin:1em 0}input,textarea,select,button{box-sizing:border-box;width:100%;padding:.6em}</style>"
        "<h1>AI Quota Monitor</h1><p>Settings are saved unencrypted on the device. Change them later with Settings &gt; Reconfigure Wi-Fi on the display. "
        "Use broker read-only device credentials, never provider tokens. HTTP setup is protected by the temporary WPA2 AP, not HTTPS. "
        "Plain MQTT exposes traffic on the LAN.</p>"
        "<form method=post action=/configure autocomplete=off><input type=hidden name=token value='";
    static constexpr char fields[] =
        "'><label>Wi-Fi SSID<input name=ssid maxlength=32 required></label>"
        "<label>Wi-Fi password (blank only for an open network)<input type=password name=wifi_password maxlength=64></label>"
        "<label>MQTT hostname or IP<input name=mqtt_host maxlength=253 required></label>"
        "<label>MQTT port<input type=number name=mqtt_port min=1 max=65535 value=1883 required></label>"
        "<label>Read-only MQTT username<input name=mqtt_username maxlength=128 required></label>"
        "<label>MQTT password<input type=password name=mqtt_password maxlength=256 required></label>"
        "<label>MQTT transport<select name=tls><option value=0>Plain TCP (trusted LAN only)</option><option value=1>TLS with CA verification</option></select></label>"
        "<label>CA certificate PEM (required for TLS; blank for TCP)<textarea name=ca maxlength=8192 rows=6></textarea></label>"
        "<button>Connect</button></form></html>";
    if (httpd_resp_send_chunk(request, prefix, sizeof(prefix) - 1) != ESP_OK ||
        httpd_resp_send_chunk(request, csrf_token, std::strlen(csrf_token)) != ESP_OK ||
        httpd_resp_send_chunk(request, fields, sizeof(fields) - 1) != ESP_OK) return ESP_FAIL;
    return httpd_resp_send_chunk(request, nullptr, 0);
}

esp_err_t configure(httpd_req_t* request) {
    if (!from_setup_interface(request)) return respond(request, "403 Forbidden", "Setup is available only on the temporary AP.");
    if (stage.load() != Stage::Setup) return respond(request, "409 Conflict", "Connection in progress. Check the display.");
    char content_type[64]{};
    if (httpd_req_get_hdr_value_str(request, "Content-Type", content_type, sizeof(content_type)) != ESP_OK ||
        std::strcmp(content_type, "application/x-www-form-urlencoded") != 0) {
        return respond(request, "415 Unsupported Media Type", "Use the device setup form.");
    }
    const size_t length = request->content_len;
    if (length == 0 || length > quota::device::kMaxFormBytes) return respond(request, "413 Content Too Large", "Setup form is too large.");
    auto erase_body = [length](char* bytes) { if (bytes) { clear_sensitive(bytes, length); delete[] bytes; } };
    std::unique_ptr<char[], decltype(erase_body)> body(new (std::nothrow) char[length], erase_body);
    auto erase_credentials = [](Credentials* value) { if (value) { clear_sensitive(value, sizeof(*value)); delete value; } };
    std::unique_ptr<Credentials, decltype(erase_credentials)> candidate(new (std::nothrow) Credentials, erase_credentials);
    if (!body || !candidate) return respond(request, "503 Service Unavailable", "Not enough memory. Retry later.");
    size_t received = 0;
    const int64_t deadline = milliseconds() + 10000;
    while (received < length) {
        if (milliseconds() >= deadline) return respond(request, "408 Request Timeout", "Setup request timed out.");
        const int count = httpd_req_recv(request, body.get() + received, length - received);
        if (count == HTTPD_SOCK_ERR_TIMEOUT) continue;
        if (count <= 0) return ESP_FAIL;
        received += static_cast<size_t>(count);
    }
    if (!quota::device::parse_provisioning(std::string_view(body.get(), length), *candidate)) {
        return respond(request, "400 Bad Request", "Invalid setup fields. Check lengths, port, password and TLS CA.");
    }
    // Constant-time token comparison rejects cross-site form submissions.
    unsigned mismatch = 0;
    for (size_t i = 0; i < 32; ++i) mismatch |= static_cast<unsigned char>(candidate->token[i] ^ csrf_token[i]);
    if (mismatch) return respond(request, "403 Forbidden", "Reload the device setup form.");
    if (candidate->tls) {
        mbedtls_x509_crt certificate;
        mbedtls_x509_crt_init(&certificate);
        const int error = mbedtls_x509_crt_parse(&certificate, reinterpret_cast<const unsigned char*>(candidate->ca), std::strlen(candidate->ca) + 1);
        mbedtls_x509_crt_free(&certificate);
        if (error != 0) return respond(request, "400 Bad Request", "CA certificate is not valid PEM.");
    }
    bool accepted = false;
    {
        UiLock lock;
        if (stage.load() == Stage::Setup) {
            clear_sensitive(candidate->token, sizeof(candidate->token));
            credentials = *candidate;
            stage.store(Stage::Connecting);
            provision_requested.store(true);
            accepted = true;
        }
    }
    if (!accepted) return respond(request, "409 Conflict", "Connection in progress. Check the display.");
    return respond(request, "202 Accepted", "Settings accepted and saved on the device. Check the device display. Setup AP stops after Wi-Fi connects.");
}

void start_http() {
    httpd_config_t configuration = HTTPD_DEFAULT_CONFIG();
    configuration.max_open_sockets = 4;
    configuration.max_uri_handlers = 2;
    configuration.max_resp_headers = 6;
    configuration.recv_wait_timeout = 2;
    configuration.send_wait_timeout = 2;
    configuration.lru_purge_enable = true;
    configuration.stack_size = 6144;
    ESP_ERROR_CHECK(httpd_start(&http_server, &configuration));
    httpd_uri_t get{};
    get.uri = "/";
    get.method = HTTP_GET;
    get.handler = setup_form;
    ESP_ERROR_CHECK(httpd_register_uri_handler(http_server, &get));
    httpd_uri_t post{};
    post.uri = "/configure";
    post.method = HTTP_POST;
    post.handler = configure;
    ESP_ERROR_CHECK(httpd_register_uri_handler(http_server, &post));
}

void start_setup_ap() {
    // Starting STA without connecting enables the RF entropy source before any
    // AP secrets are generated. No setup AP exists until its WPA2 key is ready.
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_start());
    uint8_t mac[6]{};
    ESP_ERROR_CHECK(esp_wifi_get_mac(WIFI_IF_AP, mac));
    std::snprintf(ap_ssid, sizeof(ap_ssid), "Usage-Setup-%02X%02X%02X", mac[3], mac[4], mac[5]);
    // Rejection sampling avoids bias in the random eight-digit WPA2 password.
    for (size_t i = 0; i < 8; ++i) {
        uint32_t random;
        do { random = esp_random(); } while (random >= UINT32_MAX - (UINT32_MAX % 10));
        ap_password[i] = static_cast<char>('0' + random % 10);
    }
    for (size_t i = 0; i < 16; ++i) {
        const uint8_t random = static_cast<uint8_t>(esp_random());
        std::snprintf(csrf_token + 2 * i, 3, "%02x", random);
    }
    ESP_ERROR_CHECK(esp_wifi_stop());
    wifi_config_t ap{};
    std::memcpy(ap.ap.ssid, ap_ssid, std::strlen(ap_ssid));
    std::memcpy(ap.ap.password, ap_password, 8);
    ap.ap.ssid_len = std::strlen(ap_ssid);
    ap.ap.authmode = WIFI_AUTH_WPA2_PSK;
    ap.ap.max_connection = 1;
    ap.ap.channel = 1;
    ap.ap.pmf_cfg.capable = true;
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_APSTA));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_AP, &ap));
    clear_sensitive(&ap, sizeof(ap));
    ESP_ERROR_CHECK(esp_wifi_start());
    start_http();
    UiLock lock;
    ui.setup(ap_ssid, ap_password, kSetupAddress);
}

void initialize_wifi() {
    // Wi-Fi library persistence stays off: Device setup is stored by this firmware
    // in its own NVS namespace (ADR 0002), not by the driver.
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    if (!esp_netif_create_default_wifi_sta() || !esp_netif_create_default_wifi_ap()) ESP_ERROR_CHECK(ESP_ERR_NO_MEM);
    ESP_ERROR_CHECK(esp_event_handler_register(WIFI_EVENT, ESP_EVENT_ANY_ID, wifi_event, nullptr));
    ESP_ERROR_CHECK(esp_event_handler_register(IP_EVENT, IP_EVENT_STA_GOT_IP, wifi_event, nullptr));
    wifi_init_config_t configuration = WIFI_INIT_CONFIG_DEFAULT();
    configuration.nvs_enable = 0;
    ESP_ERROR_CHECK(esp_wifi_init(&configuration));
    ESP_ERROR_CHECK(esp_wifi_set_storage(WIFI_STORAGE_RAM));
    uint8_t mac[6]{};
    ESP_ERROR_CHECK(esp_read_mac(mac, ESP_MAC_WIFI_STA));
    std::snprintf(client_id, sizeof(client_id), "quota-device-%02x%02x%02x%02x%02x%02x", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}

void apply_credentials(bool clear_password) {
    wifi_config_t station{};
    std::memcpy(station.sta.ssid, credentials.ssid, std::strlen(credentials.ssid));
    std::memcpy(station.sta.password, credentials.wifi_password, std::strlen(credentials.wifi_password));
    station.sta.threshold.authmode = credentials.wifi_password[0] ? WIFI_AUTH_WPA2_PSK : WIFI_AUTH_OPEN;
    station.sta.pmf_cfg.capable = true;
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &station));
    clear_sensitive(&station, sizeof(station));
    // The driver retains its RAM copy. Keep ours only until new values are saved after the first join.
    if (clear_password) clear_sensitive(credentials.wifi_password, sizeof(credentials.wifi_password));
    station_lost.store(false);
}

void update_settings(const char* notice) {
    char text[320]{};
    const bool online = station_online.load();
    const Stage current = stage.load();
    UiLock lock;
    std::snprintf(text, sizeof(text), "Wi-Fi: %s\nMQTT: %s\nClock: %s\nTimezone: %s\nDisplay: %s, %d%%",
        online ? "connected" : current == Stage::Connecting ? "connecting" : current == Stage::Setup ? "setup AP" : "reconnecting",
        model.mqtt_connected ? "connected" : credentials.tls && !trusted_clock.load() && current == Stage::Online ? "waiting for clock" : "offline",
        trusted_clock.load() ? "SNTP synchronized" : "Syncing clock",
        model.config.timezone.data(), quota::layout_name(model.config.layout), model.config.brightness);
    ui.settings(text);
    ui.notice(notice);
}

// Stop the station and broker connection and reopen the setup AP like a first boot.
// erase_stored also wipes the Device setup saved in NVS (Reconfigure Wi-Fi).
void restart_setup(bool erase_stored) {
    if (mqtt) {
        // A clean disconnect does not fire the Last Will, so say offline explicitly.
        esp_mqtt_client_publish(mqtt, kStatusTopic, kStatusOffline, 0, 1, 1);
        vTaskDelay(pdMS_TO_TICKS(300));
        esp_mqtt_client_stop(mqtt);
        esp_mqtt_client_destroy(mqtt);
        mqtt = nullptr;
    }
    if (erase_stored) erase_stored_credentials();
    wifi_config_t empty{};
    esp_wifi_disconnect();
    esp_wifi_set_config(WIFI_IF_STA, &empty);
    esp_wifi_stop();
    if (http_server) { httpd_stop(http_server); http_server = nullptr; }
    station_online.store(false);
    station_lost.store(false);
    {
        UiLock lock;
        clear_sensitive(&credentials, sizeof(credentials));
        model.mqtt_connected = false;
        stage.store(Stage::Setup);
    }
    start_setup_ap();
}
} // namespace

extern "C" void app_main() {
    // Disable SDK logs before any credentials arrive, including transport logs.
    esp_log_level_set("*", ESP_LOG_NONE);
    ui_mutex = xSemaphoreCreateMutex();
    if (!ui_mutex) ESP_ERROR_CHECK(ESP_ERR_NO_MEM);
    init_nvs();
    if (quota::valid_timezone(CONFIG_QUOTA_TIMEZONE)) std::strcpy(model.config.timezone.data(), CONFIG_QUOTA_TIMEZONE);
    load_config(model.config);
    applied_config = model.config;
    setenv("TZ", model.config.timezone.data(), 1);
    tzset();
    lv_init();
    board_init(model.config.brightness);
    {
        UiLock lock;
        ui.create();
    }
    initialize_wifi();

    int attempts = 0;
    int64_t next_connect = 0, connect_deadline = 0, settings_due = 0, mqtt_due = 0;
    int64_t previous_tick = milliseconds();
    bool connecting = false, first_ip = false, sntp_started = false, have_credentials = false, join_failed = false;
    if (load_credentials(credentials)) {
        // Device setup survived the reboot: join Wi-Fi without opening the setup AP.
        stage.store(Stage::Connecting);
        ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
        ESP_ERROR_CHECK(esp_wifi_start());
        credentials_persisted = true;
        apply_credentials(true);
        have_credentials = true;
    } else {
        start_setup_ap();
    }
    while (true) {
        const int64_t now = milliseconds();
        if (provision_requested.exchange(false)) {
            // Not stored yet: only a successful join persists these values.
            credentials_persisted = false;
            join_failed = false;
            apply_credentials(false);
            have_credentials = true;
            attempts = 0;
            connecting = false;
            first_ip = false;
            // Let the HTTP success response reach the phone before switching channels.
            next_connect = now + 1000;
        }
        bool reconfigure_requested;
        {
            UiLock lock;
            reconfigure_requested = ui.take_reconfigure_request();
        }
        if (reconfigure_requested) {
            restart_setup(true);
            have_credentials = false;
            attempts = 0;
            connecting = false;
            first_ip = false;
            join_failed = false;
        }
        if (have_credentials && stage.load() != Stage::Setup) {
            if (station_online.load()) {
                connecting = false;
                attempts = 0;
                if (!first_ip) {
                    first_ip = true;
                    if (!credentials_persisted) {
                        // Wi-Fi accepted the submitted values: now they are worth keeping across reboots.
                        save_credentials(credentials);
                        credentials_persisted = true;
                        clear_sensitive(credentials.wifi_password, sizeof(credentials.wifi_password));
                    }
                    stage.store(Stage::Online);
                    if (http_server) {
                        // First join after Device setup: the setup AP is no longer needed.
                        ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
                        httpd_stop(http_server);
                        http_server = nullptr;
                        clear_sensitive(ap_password, sizeof(ap_password));
                        clear_sensitive(csrf_token, sizeof(csrf_token));
                        UiLock lock;
                        ui.setup(nullptr, nullptr, nullptr);
                    }
                    if (!sntp_started) {
                        esp_sntp_setoperatingmode(ESP_SNTP_OPMODE_POLL);
                        esp_sntp_setservername(0, CONFIG_QUOTA_NTP_SERVER);
                        esp_sntp_set_time_sync_notification_cb(time_sync);
                        esp_sntp_init();
                        sntp_started = true;
                    }
                }
                if (now >= mqtt_due) { start_mqtt(); mqtt_due = now + 5000; }
            } else {
                if (station_lost.exchange(false) || (connecting && now >= connect_deadline)) {
                    esp_wifi_disconnect();
                    connecting = false;
                    const int delay = std::min(30, 1 << std::min(attempts, 5));
                    next_connect = now + delay * 1000;
                    // Avoid treating our deliberate disconnect as another failed attempt.
                    station_lost.store(false);
                }
                if (!connecting && now >= next_connect) {
                    if (quota::device::abandon_unsaved_setup(credentials_persisted, attempts)) {
                        // Just-submitted values never joined: forget them (they were never stored) and let the user retry.
                        restart_setup(false);
                        have_credentials = false;
                        attempts = 0;
                        connecting = false;
                        first_ip = false;
                        join_failed = true;
                    } else {
                        // Saved Device setup is never discarded automatically: bounded backoff forever.
                        attempts = std::min(attempts + 1, 6);
                        if (esp_wifi_connect() == ESP_OK) {
                            connecting = true;
                            connect_deadline = now + 15000;
                        } else {
                            next_connect = now + 5000;
                        }
                    }
                }
            }
        }
        quota::DeviceConfig wanted;
        {
            UiLock lock;
            wanted = model.config;
        }
        if (!(wanted == applied_config)) {
            if (wanted.timezone != applied_config.timezone) {
                setenv("TZ", wanted.timezone.data(), 1);
                tzset();
            }
            if (wanted.brightness != applied_config.brightness) board_set_brightness(wanted.brightness);
            save_config(wanted);
            applied_config = wanted;
        }
        if (now >= settings_due) {
            const char* notice = join_failed ? "Could not join Wi-Fi. Check the password and try again."
                : have_credentials && credentials_persisted && !station_online.load() && attempts >= 3 ? "Cannot join Wi-Fi. Open Settings > Reconfigure Wi-Fi."
                : nullptr;
            update_settings(notice);
            settings_due = now + 1000;
        }
        const int64_t current_tick = milliseconds();
        {
            UiLock lock;
            lv_tick_inc(static_cast<uint32_t>(current_tick - previous_tick));
            ui.tick(current_tick);
            lv_timer_handler();
        }
        previous_tick = current_tick;
        vTaskDelay(pdMS_TO_TICKS(10));
    }
}
