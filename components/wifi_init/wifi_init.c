#include "esp_wifi.h"

_Static_assert(WIFI_TASK_STACK_SIZE > 0, "ESP-IDF must provide a nonzero Wi-Fi task stack");

esp_err_t __real_esp_wifi_init(const wifi_init_config_t *config);

// esp-idf-svc 0.53.0 leaves wifi_task_stack_size at zero.
// Keep the driver's NVS policy and take the complete configuration from ESP-IDF.
esp_err_t __wrap_esp_wifi_init(const wifi_init_config_t *config)
{
    wifi_init_config_t defaults = WIFI_INIT_CONFIG_DEFAULT();
    defaults.nvs_enable = config->nvs_enable;
    return __real_esp_wifi_init(&defaults);
}
