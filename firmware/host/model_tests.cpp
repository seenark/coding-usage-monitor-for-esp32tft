#include "quota_model.hpp"
#include "cJSON.h"
#include <cassert>
#include <fstream>
#include <iterator>
#include <memory>
#include <string>
#include <iostream>

namespace {
using Json = std::unique_ptr<cJSON, decltype(&cJSON_Delete)>;
constexpr const char* codex_topic = "ai-quota/v1/providers/codex";
constexpr const char* claude_topic = "ai-quota/v1/providers/claude";
std::string load(const char* name) {
    std::ifstream input(std::string(QUOTA_FIXTURE_DIR) + "/" + name);
    assert(input);
    return {std::istreambuf_iterator<char>(input),std::istreambuf_iterator<char>()};
}
Json parse(const std::string& input) { Json value(cJSON_Parse(input.c_str()),cJSON_Delete); assert(value); return value; }
std::string encode(cJSON* value) { auto text = cJSON_PrintUnformatted(value); assert(text); std::string out(text); cJSON_free(text); return out; }
void number(cJSON* value, const char* key, double number) { assert(cJSON_ReplaceItemInObjectCaseSensitive(value,key,cJSON_CreateNumber(number))); }
void string(cJSON* value, const char* key, const char* text) { assert(cJSON_ReplaceItemInObjectCaseSensitive(value,key,cJSON_CreateString(text))); }
void null(cJSON* value, const char* key) { assert(cJSON_ReplaceItemInObjectCaseSensitive(value,key,cJSON_CreateNull())); }
}
int main() {
    using namespace quota;
    const auto codex = load("codex-v1.json"), claude = load("claude-v1.json");
    Model model;
    assert(model.ingest(codex_topic,codex) == ApplyResult::Applied);
    assert(model.ingest(claude_topic,claude) == ApplyResult::Applied);
    const auto snapshot = model.snapshot(Provider::Codex);
    assert(snapshot && snapshot->window_count == 3);
    assert(snapshot->windows[0].remaining_percent == 0);
    assert(snapshot->windows[2].duration_minutes == 73);
    assert(snapshot->windows[2].used_percent == 120 && snapshot->windows[2].remaining_percent == 0);
    assert(snapshot->banked_resets->available_count == 8 && snapshot->banked_resets->details_status == Details::Partial);
    assert(snapshot->banked_resets->known_nonexpiring_count == 0);
    const auto base = snapshot->published_at;
    assert(model.ingest(codex_topic,codex) == ApplyResult::Ignored);
    assert(model.ingest(claude_topic,codex) == ApplyResult::Invalid);
    assert(model.ingest("ai-quota/v1/providers/other",codex) == ApplyResult::Invalid);
    assert(model.ingest(codex_topic,codex + "garbage") == ApplyResult::Invalid);
    assert(model.ingest(codex_topic,std::string(max_snapshot_bytes + 1,' ')) == ApplyResult::Invalid);
    assert(model.ingest(codex_topic,std::string(13,'[') + std::string(13,']')) == ApplyResult::Invalid);
    assert(model.ingest(codex_topic,"{\"schema_version\":1,\"schema_version\":1}") == ApplyResult::Invalid);
    assert(model.ingest(codex_topic,"{\"provider\":\"codex\\u0000evil\"}") == ApplyResult::Invalid);
    auto updated = parse(codex);
    number(updated.get(),"published_at",base - 1);
    assert(model.ingest(codex_topic,encode(updated.get())) == ApplyResult::Ignored);
    std::string invalid_utf8 = "{\"provider\":\"";
    invalid_utf8.push_back(static_cast<char>(0xC0));
    invalid_utf8 += "\"}";
    assert(model.ingest(codex_topic,invalid_utf8) == ApplyResult::Invalid);
    number(updated.get(),"published_at",base + 2);
    number(updated.get(),"fetched_at",base - 1);
    assert(model.ingest(codex_topic,encode(updated.get())) == ApplyResult::Ignored);
    number(updated.get(),"fetched_at",base);
    string(updated.get(),"state","reauth_required"); string(updated.get(),"error_code","unauthenticated");
    assert(model.ingest(codex_topic,encode(updated.get())) == ApplyResult::Applied);
    assert(model.snapshot(Provider::Claude)->state == State::Ok);
    assert(model.snapshot(Provider::Codex)->state == State::ReauthRequired);
    assert(model.snapshot(Provider::Codex)->fetched_at == base);
    number(updated.get(),"published_at",base + 3);
    null(updated.get(),"fetched_at");
    assert(cJSON_ReplaceItemInObjectCaseSensitive(updated.get(),"windows",cJSON_CreateArray()));
    null(updated.get(),"banked_resets");
    assert(model.ingest(codex_topic,encode(updated.get())) == ApplyResult::Applied);
    assert(model.snapshot(Provider::Codex)->window_count == 3 && model.snapshot(Provider::Codex)->fetched_at == base);
    assert(model.ingest("ai-quota/v1/server/status","{\"state\":\"online\"}") == ApplyResult::Applied);
    assert(model.ingest("ai-quota/v1/server/status","{\"state\":\"online\"}") == ApplyResult::Ignored);
    model.mqtt_connected = true;
    assert(badge(model.snapshot(Provider::Codex),model,base + 901).find("STALE") != std::string::npos);
    model.mqtt_connected = false;
    assert(badge(model.snapshot(Provider::Claude),model,base).find("OFFLINE") != std::string::npos);
    assert(badge(model.snapshot(Provider::Claude),model,base).find("DEMO") != std::string::npos);
    Clock clock;
    assert(!clock.now(1000));
    clock.sync(base,1000);
    assert(clock.now(1999) == base);
    assert(clock.now(3000) == base + 2);
    assert(!clock.now(999));
    clock.sync(base - 100,4000); // NTP can correct a wall clock backwards.
    assert(clock.now(5000) == base - 99);
    clock.clear(); assert(!clock.now(6000));
    assert(!countdown(std::nullopt,base)); assert(!countdown(base,std::nullopt));
    assert(countdown(base + 5,base) == 5);
    assert(countdown(base,base) == 0 && countdown(base - 1,base) == 0);
    assert(countdown(0,0) == 0); // Epoch zero is a known timestamp, not null.
    assert(!freshness(*snapshot,std::nullopt).age_seconds);
    assert(!freshness(*snapshot,base + 899).stale);
    assert(freshness(*snapshot,base + 900).stale);
    assert(freshness(*snapshot,base - 61).clock_skew);
    assert(!freshness(*snapshot,base - 60).clock_skew);
    assert(freshness(*snapshot,base + 86400).stale);
    assert(snapshot->windows[0].remaining_percent == 0); // Crossing reset never restores quota locally.
    auto unknown = parse(codex);
    null(unknown.get(),"fetched_at"); null(unknown.get(),"banked_resets");
    auto windows = cJSON_GetObjectItemCaseSensitive(unknown.get(),"windows");
    auto first = cJSON_GetArrayItem(windows,0);
    null(first,"used_percent"); null(first,"remaining_percent"); null(first,"duration_minutes"); null(first,"resets_at");
    Model empty;
    assert(empty.ingest(codex_topic,encode(unknown.get())) == ApplyResult::Applied);
    assert(!empty.snapshot(Provider::Codex)->fetched_at && !empty.snapshot(Provider::Codex)->banked_resets);
    assert(!empty.snapshot(Provider::Codex)->windows[0].remaining_percent);
    auto zero = parse(codex);
    auto bank = cJSON_GetObjectItemCaseSensitive(zero.get(),"banked_resets");
    number(bank,"available_count",0); string(bank,"details_status","complete");
    assert(cJSON_ReplaceItemInObjectCaseSensitive(bank,"known_expirations_at",cJSON_CreateArray()));
    Model zeros; assert(zeros.ingest(codex_topic,encode(zero.get())) == ApplyResult::Applied);
    assert(zeros.snapshot(Provider::Codex)->banked_resets->available_count == 0);
    auto capped = parse(codex);
    bank = cJSON_GetObjectItemCaseSensitive(capped.get(),"banked_resets");
    auto omitted = parse(codex);
    cJSON_DeleteItemFromObjectCaseSensitive(omitted.get(),"source");
    cJSON_DeleteItemFromObjectCaseSensitive(omitted.get(),"fetched_at");
    cJSON_DeleteItemFromObjectCaseSensitive(omitted.get(),"banked_resets");
    cJSON_DeleteItemFromObjectCaseSensitive(omitted.get(),"error_code");
    first = cJSON_GetArrayItem(cJSON_GetObjectItemCaseSensitive(omitted.get(),"windows"),0);
    for (const auto key : {"used_percent","remaining_percent","duration_minutes","resets_at"})
        cJSON_DeleteItemFromObjectCaseSensitive(first,key);
    Model missing;
    assert(missing.ingest(codex_topic,encode(omitted.get())) == ApplyResult::Applied);
    assert(!missing.snapshot(Provider::Codex)->mock);
    assert(!missing.snapshot(Provider::Codex)->windows[0].resets_at);
    number(bank,"available_count",100);
    auto dates = cJSON_GetObjectItemCaseSensitive(bank,"known_expirations_at");
    while (cJSON_GetArraySize(dates)) cJSON_DeleteItemFromArray(dates,0);
    for (size_t i = 0; i < max_expirations; ++i) cJSON_AddItemToArray(dates,cJSON_CreateNumber(static_cast<double>(base + i)));
    Model cap; assert(cap.ingest(codex_topic,encode(capped.get())) == ApplyResult::Applied);
    assert(cap.snapshot(Provider::Codex)->banked_resets->expiration_count == 64);
    assert(cap.snapshot(Provider::Codex)->banked_resets->details_status == Details::Partial);
    cJSON_AddItemToArray(dates,cJSON_CreateNumber(static_cast<double>(base)));
    assert(cap.ingest(codex_topic,encode(capped.get())) == ApplyResult::Invalid);
    auto invalid = parse(codex);
    first = cJSON_GetArrayItem(cJSON_GetObjectItemCaseSensitive(invalid.get(),"windows"),0);
    number(first,"remaining_percent",100);
    assert(cap.ingest(codex_topic,encode(invalid.get())) == ApplyResult::Invalid);
    number(first,"remaining_percent",0); number(first,"resets_at",0);
    number(invalid.get(),"published_at",base + 10);
    assert(cap.ingest(codex_topic,encode(invalid.get())) == ApplyResult::Applied);
    assert(cap.snapshot(Provider::Codex)->windows[0].resets_at == 0);
    auto excessive = parse(codex);
    windows = cJSON_GetObjectItemCaseSensitive(excessive.get(),"windows");
    while (cJSON_GetArraySize(windows) <= 32) cJSON_AddItemToArray(windows,cJSON_Duplicate(cJSON_GetArrayItem(windows,0),true));
    assert(cap.ingest(codex_topic,encode(excessive.get())) == ApplyResult::Invalid);
    auto live = parse(codex);
    string(live.get(),"source","provider"); string(live.get(),"state","unavailable");
    number(live.get(),"published_at",base - 1); null(live.get(),"fetched_at");
    assert(cJSON_ReplaceItemInObjectCaseSensitive(live.get(),"windows",cJSON_CreateArray()));
    null(live.get(),"banked_resets");
    assert(model.ingest(codex_topic,encode(live.get())) == ApplyResult::Applied);
    assert(!model.snapshot(Provider::Codex)->mock);
    assert(!model.snapshot(Provider::Codex)->fetched_at && model.snapshot(Provider::Codex)->window_count == 0);
    assert(model.ingest(codex_topic,codex) == ApplyResult::Ignored);
    auto authoritative = parse(codex);
    bank = cJSON_GetObjectItemCaseSensitive(authoritative.get(),"banked_resets");
    number(bank,"available_count",0);
    Model counts;
    assert(counts.ingest(codex_topic,encode(authoritative.get())) == ApplyResult::Applied);
    assert(counts.snapshot(Provider::Codex)->banked_resets->available_count == 0);
    assert(counts.snapshot(Provider::Codex)->banked_resets->expiration_count == 3);
    auto count_only = parse(codex);
    bank = cJSON_GetObjectItemCaseSensitive(count_only.get(),"banked_resets");
    null(bank,"available_count"); string(bank,"details_status","unavailable");
    assert(cJSON_ReplaceItemInObjectCaseSensitive(bank,"known_expirations_at",cJSON_CreateArray()));
    Model unavailable;
    assert(unavailable.ingest(codex_topic,encode(count_only.get())) == ApplyResult::Applied);
    assert(!unavailable.snapshot(Provider::Codex)->banked_resets->available_count);
    assert(unavailable.snapshot(Provider::Codex)->banked_resets->details_status == Details::Unavailable);
    assert(unavailable.snapshot(Provider::Codex)->banked_resets->known_nonexpiring_count == 0);
    // device/config: decoded per field. A bad field keeps its previous value, never the whole payload.
    Model device;
    const std::string config_topic = device_config_topic;
    assert(device.config.layout == Layout::Cards && std::string(device.config.timezone.data()) == "ICT-7" && device.config.brightness == 80);
    assert(device.ingest(config_topic,R"({"schema_version":1,"layout":"gauges","timezone":"CET-1CEST,M3.5.0,M10.5.0/3","brightness":55})") == ApplyResult::Applied);
    assert(device.config.layout == Layout::Gauges && std::string(device.config.timezone.data()) == "CET-1CEST,M3.5.0,M10.5.0/3" && device.config.brightness == 55);
    assert(device.ingest(config_topic,R"({"schema_version":1,"layout":"gauges","timezone":"CET-1CEST,M3.5.0,M10.5.0/3","brightness":55})") == ApplyResult::Ignored);
    // Partial payload: only the named field changes.
    assert(device.ingest(config_topic,R"({"schema_version":1,"layout":"terminal"})") == ApplyResult::Applied);
    assert(device.config.layout == Layout::Terminal && device.config.brightness == 55 && std::string(device.config.timezone.data()) == "CET-1CEST,M3.5.0,M10.5.0/3");
    // Unknown layout, wrong types and bad timezones keep the old values while good fields still apply.
    assert(device.ingest(config_topic,R"({"schema_version":1,"layout":"holo","timezone":"bad zone!","brightness":70})") == ApplyResult::Applied);
    assert(device.config.layout == Layout::Terminal && std::string(device.config.timezone.data()) == "CET-1CEST,M3.5.0,M10.5.0/3" && device.config.brightness == 70);
    assert(device.ingest(config_topic,R"({"schema_version":1,"layout":7,"timezone":"","brightness":"90"})") == ApplyResult::Ignored);
    assert(device.ingest(config_topic,R"({"schema_version":1,"timezone":")" + std::string(65,'A') + R"("})") == ApplyResult::Ignored);
    assert(device.config.brightness == 70);
    // Brightness: integers are clamped to 10..=100, anything else is ignored.
    assert(device.ingest(config_topic,R"({"schema_version":1,"brightness":3})") == ApplyResult::Applied && device.config.brightness == 10);
    assert(device.ingest(config_topic,R"({"schema_version":1,"brightness":250})") == ApplyResult::Applied && device.config.brightness == 100);
    assert(device.ingest(config_topic,R"({"schema_version":1,"brightness":42.5})") == ApplyResult::Ignored && device.config.brightness == 100);
    assert(device.ingest(config_topic,R"({"schema_version":1,"brightness":null})") == ApplyResult::Ignored && device.config.brightness == 100);
    // Wrong or missing schema_version, malformed JSON and duplicate keys reject the whole payload.
    assert(device.ingest(config_topic,R"({"schema_version":2,"layout":"cards","brightness":20})") == ApplyResult::Invalid);
    assert(device.ingest(config_topic,R"({"layout":"cards","brightness":20})") == ApplyResult::Invalid);
    assert(device.ingest(config_topic,R"({"schema_version":1,"brightness":20,"brightness":30})") == ApplyResult::Invalid);
    assert(device.ingest(config_topic,"[]") == ApplyResult::Invalid);
    assert(device.ingest(config_topic,"{") == ApplyResult::Invalid);
    assert(device.config.layout == Layout::Terminal && device.config.brightness == 100);
    assert(valid_timezone("ICT-7") && valid_timezone("<+07>-7") && !valid_timezone("") && !valid_timezone("a b") && !valid_timezone(std::string(65,'A')));
    std::cout << "Firmware model checks passed\n";
}
