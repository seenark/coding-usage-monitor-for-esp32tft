idf_component_register(
    SRCS "runtime.cpp" "board.cpp" "provision.cpp"
         "../shared/quota_model.cpp" "../shared/quota_ui.cpp"
    INCLUDE_DIRS "." "../shared"
    REQUIRES lvgl json esp_wifi esp_event esp_netif mqtt esp_http_server
             nvs_flash esp_timer driver esp_lcd lwip mbedtls
)
target_compile_features(${COMPONENT_LIB} PUBLIC cxx_std_17)
