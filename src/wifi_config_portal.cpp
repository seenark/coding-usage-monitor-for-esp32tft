#include "wifi_config_portal.h"

namespace {
constexpr const char *AP_NAME = "CodexMonitor-Setup";
constexpr const char *NVS_NAMESPACE = "wifi_setup";
}

void WifiConfigPortal::begin() {
  preferences_.begin(NVS_NAMESPACE, false);
  ssid_ = preferences_.getString("ssid", "");
  password_ = preferences_.getString("password", "");
  WiFi.mode(WIFI_AP_STA);
  WiFi.softAP(AP_NAME);
  server_.begin();
  portal_started_ = true;
  Serial.printf("wifi setup AP=%s IP=%s credentials=%s\n", AP_NAME,
                WiFi.softAPIP().toString().c_str(), has_credentials() ? "saved" : "empty");
}

void WifiConfigPortal::tick() {
  if (!portal_started_) return;
  WiFiClient client = server_.available();
  if (client) handle_client(client);
}

bool WifiConfigPortal::has_credentials() const {
  return !ssid_.isEmpty() && !password_.isEmpty();
}

const String &WifiConfigPortal::ssid() const { return ssid_; }
const String &WifiConfigPortal::password() const { return password_; }

String WifiConfigPortal::decode_form_value(const String &value) {
  String decoded;
  decoded.reserve(value.length());
  for (size_t index = 0; index < value.length(); ++index) {
    if (value[index] == '+') {
      decoded += ' ';
    } else if (value[index] == '%' && index + 2 < value.length()) {
      const char hex[] = {value[index + 1], value[index + 2], '\0'};
      decoded += static_cast<char>(strtol(hex, nullptr, 16));
      index += 2;
    } else {
      decoded += value[index];
    }
  }
  return decoded;
}

void WifiConfigPortal::handle_client(WiFiClient &client) {
  const uint32_t deadline = millis() + 1500;
  String request;
  while (client.connected() && millis() < deadline) {
    while (client.available()) {
      request += static_cast<char>(client.read());
    }
    const int header_end = request.indexOf("\r\n\r\n");
    if (header_end < 0) {
      delay(1);
      continue;
    }
    const int length_start = request.indexOf("Content-Length:");
    const int content_length =
        length_start >= 0 ? request.substring(length_start + 15).toInt() : 0;
    if (request.length() >= header_end + 4 + content_length) break;
    delay(1);
  }

  const int body_start = request.indexOf("\r\n\r\n");
  const String body = body_start >= 0 ? request.substring(body_start + 4) : String();
  if (request.startsWith("POST /wifi") && !body.isEmpty()) {
    const int ssid_start = body.indexOf("ssid=");
    const int password_start = body.indexOf("password=");
    if (ssid_start >= 0 && password_start >= 0) {
      const int ssid_end = body.indexOf('&', ssid_start);
      const String raw_ssid = body.substring(ssid_start + 5, ssid_end >= 0 ? ssid_end : body.length());
      const String raw_password = body.substring(password_start + 9);
      ssid_ = decode_form_value(raw_ssid);
      password_ = decode_form_value(raw_password);
      if (!ssid_.isEmpty() && !password_.isEmpty()) {
        preferences_.putString("ssid", ssid_);
        preferences_.putString("password", password_);
      }
    }
  }

  client.println("HTTP/1.1 200 OK");
  client.println("Content-Type: text/html; charset=utf-8");
  client.println("Connection: close");
  client.println();
  client.println("<!doctype html><html><body><h1>Codex Monitor Wi-Fi</h1>");
  client.println("<form method='POST' action='/wifi'>");
  client.println("SSID: <input name='ssid'><br>Password: <input name='password' type='password'><br>");
  client.println("<button>Save Wi-Fi</button></form>");
  client.println(has_credentials() ? "<p>Credentials saved. Restart the monitor.</p>" : "<p>No credentials saved.</p>");
  client.println("</body></html>");
  delay(2);
  client.stop();
}
