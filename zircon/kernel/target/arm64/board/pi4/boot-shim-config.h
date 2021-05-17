// Copyright 2021 The Fuchsia (and Camden) Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT
#define HAS_DEVICE_TREE 0
static const zbi_mem_range_t mem_config[] = {
    {
        .paddr =  0x000000000,
        .length = 0x40000000,  // 1GB
        .type = ZBI_MEM_RANGE_RAM,
    },
    {
        .paddr = 0x47C000000,
        .length = 0x47FFFFFFF,
        .type = ZBI_MEM_RANGE_PERIPHERAL,
    },
};

static const zbi_platform_id_t platform_id = {
    .vid = PDEV_VID_RASPBERRY,
    .pid = PDEV_PID_RASPBERRY_PI_4,
    .board_name = "Raspberry-Pi-4",
};
static void add_cpu_topology(zbi_header_t* zbi) {
#define TOPOLOGY_CPU_COUNT 4
  zbi_topology_node_t nodes[TOPOLOGY_CPU_COUNT];
  for (uint8_t index = 0; index < TOPOLOGY_CPU_COUNT; index++) {
    nodes[index] = (zbi_topology_node_t){
        .entity_type = ZBI_TOPOLOGY_ENTITY_PROCESSOR,
        .parent_index = ZBI_TOPOLOGY_NO_PARENT,
        .entity =
            {
                .processor =
                    {
                        .logical_ids = {index},
                        .logical_id_count = 1,
                        .flags = (uint16_t)(index == 0 ? ZBI_TOPOLOGY_PROCESSOR_PRIMARY : 0),
                        .architecture = ZBI_TOPOLOGY_ARCH_ARM,
                        .architecture_info =
                            {
                                .arm =
                                    {
                                        .cpu_id = index,
                                        .gic_id = index,
                                    },
                            },
                    },
            },
    };
  }
  append_boot_item(zbi, ZBI_TYPE_CPU_TOPOLOGY, sizeof(zbi_topology_node_t), &nodes,
                   sizeof(zbi_topology_node_t) * TOPOLOGY_CPU_COUNT);
}
static void append_board_boot_item(zbi_header_t* bootdata) {
  add_cpu_topology(bootdata);
  // add kernel drivers
  append_boot_item(bootdata, ZBI_TYPE_KERNEL_DRIVER, KDRV_DW8250_UART, &uart_driver,
                   sizeof(uart_driver));
  append_boot_item(bootdata, ZBI_TYPE_KERNEL_DRIVER, KDRV_ARM_GIC_V2, &gicv2_driver,
                   sizeof(gicv2_driver));
  append_boot_item(bootdata, ZBI_TYPE_KERNEL_DRIVER, KDRV_ARM_PSCI, &psci_driver,
                   sizeof(psci_driver));

  // add platform ID
  append_boot_item(bootdata, ZBI_TYPE_PLATFORM_ID, 0, &platform_id, sizeof(platform_id));
}
