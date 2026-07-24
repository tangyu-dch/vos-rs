//! Copilot 工具（Function Calling）JSON Schema 定义。
//!
//! 拆分自 copilot.rs，纯数据字面量，定义 LLM 可调用的工具集 schema。

use serde_json::json;

pub fn get_copilot_tools_schema() -> serde_json::Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "vos_get_dashboard_stats",
                "description": "获取 VoIP 软交换平台整体运行概览指标（实时 CPS 呼叫并发、接通率 ASR、平均 MOS 音质评分、活跃通话数、注册分机数等）。当用户询问系统状态、概况、集群健康度或大盘监控时调用。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_cdrs",
                "description": "条件查询呼叫详单 (CDR)。支持按通话状态 (answered/failed/canceled)、主叫号码 (caller)、被叫号码 (callee) 筛选。当用户排查具体呼叫记录或失败原因时调用。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status": { "type": "string", "description": "筛选通话状态: answered / failed / canceled", "enum": ["answered", "failed", "canceled"] },
                        "caller": { "type": "string", "description": "主叫号码过滤" },
                        "callee": { "type": "string", "description": "被叫号码过滤" },
                        "limit": { "type": "integer", "description": "返回条数上限，默认 10", "default": 10 }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_get_sip_flows",
                "description": "获取指定通话 (call_id) 的完整 SIP 信令抓包交互数据，并自动生成 ASCII 信令交互梯形图 (SIP Ladder Diagram)。当用户要求查看抓包、绘制信令图或排查挂断流程时调用。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "call_id": { "type": "string", "description": "通话唯一的 Call-ID 字符串" }
                    },
                    "required": ["call_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_active_calls",
                "description": "查询当前软交换平台正在进行的实时并发通话列表。当用户询问‘现在有哪些通话’、‘实时并发数’或排查通道挂起时调用。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_terminate_call",
                "description": "强制拆线挂断指定 Call-ID 的实时并发通话。高危运维动作，仅在确定需要拆断某个活跃 call_id 时调用。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "call_id": { "type": "string", "description": "需要断开的 Call-ID" }
                    },
                    "required": ["call_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_registrations",
                "description": "查询 SIP 分机终端与外部中继的实时注册上线状态。可按 username 搜索，用于诊断分机掉线或未注册问题。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "分机账号/用户名过滤" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_gateways",
                "description": "查询软交换中继网关列表及链路健康状态与通道容量。用于网关巡检或检查落地中继。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_preview_route",
                "description": "模拟呼叫选路决策测试（指定主叫 caller 与被叫 callee）。返回选路引擎算出的命中路由、出口网关及预期计费规则。用于路由试算与拨号测试。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "caller": { "type": "string", "description": "主叫号码" },
                        "callee": { "type": "string", "description": "被叫号码" }
                    },
                    "required": ["caller", "callee"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_anti_fraud_rules",
                "description": "查询防刷量、频控与反欺诈风控规则列表。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_extensions",
                "description": "获取系统 SIP 分机账号列表（支持按用户名 username 模糊检索）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "分机账号/用户名过滤" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_extension",
                "description": "单个开户创建 SIP 分机账号（指定 username 与 password）。后端包含重复冲突检测。如用户输入的是多条分机或杂乱文本，请自动清洗并改用 vos_import_extensions。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "分机账号/用户名" },
                        "password": { "type": "string", "description": "分机注册密码" }
                    },
                    "required": ["username", "password"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_extension",
                "description": "删除指定的 SIP 分机账号。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "待删除的分机账号" }
                    },
                    "required": ["username"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_ivr_menus",
                "description": "获取系统的 IVR 语音导航菜单流程列表及按键节点关系。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_ivr_menu",
                "description": "创建或更新 IVR 语音导航菜单流程（指定菜单 ID id、名称 name、绑定的 DID 号码 did、欢迎语 welcome_prompt）。后端包含 DID 重复绑定警告。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "IVR 菜单唯一标识 ID" },
                        "name": { "type": "string", "description": "IVR 菜单显示名称" },
                        "did": { "type": "string", "description": "绑定的呼入 DID 号码" },
                        "welcome_prompt": { "type": "string", "description": "欢迎提示音语音内容或路径" }
                    },
                    "required": ["id", "name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_gateway",
                "description": "创建或更新对接网关/中继线路（指定网关 ID id、名称 name、目标 IP ip_address、端口 port）。后端包含 IP 冲突与重复检测。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "网关唯一 ID" },
                        "name": { "type": "string", "description": "网关名称" },
                        "ip_address": { "type": "string", "description": "目标 IP 地址" },
                        "port": { "type": "integer", "description": "目标端口 (默认 5060)", "default": 5060 }
                    },
                    "required": ["id", "name", "ip_address"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_gateway",
                "description": "删除指定的软交换中继网关。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的网关 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_routes",
                "description": "获取系统中的所有前缀呼叫路由规则列表。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_route",
                "description": "创建或修改前缀选路路由规则（指定路由 ID id、号码前缀 prefix、目标网关 ID gateway_id、优先级 priority）。创建后自动触发 NATS 选路引擎实时重载，包含网关存在性与前缀覆盖检测。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "路由唯一 ID" },
                        "prefix": { "type": "string", "description": "号码前缀" },
                        "gateway_id": { "type": "string", "description": "目标落地网关 ID" },
                        "priority": { "type": "integer", "description": "优先级 (数字越小优先级越高)", "default": 1 }
                    },
                    "required": ["id", "prefix", "gateway_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_route",
                "description": "删除指定的前缀呼叫路由规则并刷新选路引擎。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的路由 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_billing_accounts",
                "description": "获取所有计费账户及当前余额信息。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_recharge_billing_account",
                "description": "为计费账户资金进行充值或扣款变动（指定账户账号 account_id、变动金额 amount：正数为充值，负数为扣款、备注说明 description）。后端包含行级锁与幂等校验。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "account_id": { "type": "string", "description": "计费账户账号或唯一标识" },
                        "amount": { "type": "number", "description": "充值金额（正数为充值，负数为扣款）" },
                        "description": { "type": "string", "description": "充值/扣款备注" }
                    },
                    "required": ["account_id", "amount"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_rates",
                "description": "获取系统呼叫资费费率表。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_upsert_rate",
                "description": "创建或修改呼叫资费费率（指定费率 ID id、号码前缀 prefix、每分钟费率 rate_per_minute）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "费率唯一 ID" },
                        "prefix": { "type": "string", "description": "号码前缀" },
                        "rate_per_minute": { "type": "number", "description": "每分钟费率金额" }
                    },
                    "required": ["id", "prefix", "rate_per_minute"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_rate",
                "description": "删除指定的呼叫资费费率。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的费率 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_add_ivr_node",
                "description": "向现有 IVR 菜单添加/配置按键转接节点（指定 IVR ID id、按键 dtmf_key 0-9/*/#、目标动作 action 例如 extension:8001 或 hangup）。支持自然语言指令如‘按 1 转分机 8001’。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "IVR 菜单 ID" },
                        "dtmf_key": { "type": "string", "description": "按键 (0-9, *, #, timeout)" },
                        "action": { "type": "string", "description": "转接动作或目标，例如 extension:8001" }
                    },
                    "required": ["id", "dtmf_key", "action"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_ivr_menu",
                "description": "删除指定的 IVR 语音导航菜单。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的 IVR 菜单 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_anti_fraud_rule",
                "description": "创建防刷量/高危频控风控规则（指定规则 ID id、规则名称 name、匹配模式 pattern、频控上限 limit_count）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "规则 ID" },
                        "name": { "type": "string", "description": "规则名称" },
                        "pattern": { "type": "string", "description": "匹配模式 (如 IP 网段或主叫前缀)" },
                        "limit_count": { "type": "integer", "description": "允许最大并发或频控值", "default": 60 }
                    },
                    "required": ["id", "name", "pattern"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_anti_fraud_rule",
                "description": "删除防刷量/频控风控规则。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的规则 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_export_cdrs",
                "description": "根据多条件（主叫 caller、被叫 callee、状态 status：ANSWERED/FAILED/BUSY、时间范围）筛选并导出 CDR 详单数据，返回可直接用于前端自适应环境下载的相对 API 接口路径。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "caller": { "type": "string", "description": "主叫号码筛选" },
                        "callee": { "type": "string", "description": "被叫号码筛选" },
                        "status": { "type": "string", "description": "呼叫状态 (如 ANSWERED, FAILED, BUSY)" },
                        "start_time": { "type": "string", "description": "起始时间" },
                        "end_time": { "type": "string", "description": "截止时间" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_export_extensions",
                "description": "导出全量 SIP 分机账号数据，返回环境自适应的相对 API 下载路径。",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_export_gateways",
                "description": "导出全量中继网关节点数据，返回环境自适应的相对 API 下载路径。",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_export_routes",
                "description": "导出全量前缀选路路由规则，返回环境自适应的相对 API 下载路径。",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_export_rates",
                "description": "导出全量呼叫资费费率表，返回环境自适应的相对 API 下载路径。",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_export_billing_accounts",
                "description": "导出全量计费账户及余额摘要，返回环境自适应的相对 API 下载路径。",
                "parameters": { "type": "object", "properties": {} }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_import_extensions",
                "description": "智能批量导入 SIP 分机账号。无论是规整的 CSV，还是杂乱的自然语言文本（如 '小王8001密码123456，小张8002密码888888'），大模型都会自动提取整理为标准 CSV ('username,password') 并通过本工具下发开户。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "由 AI 整理提取出的标准 CSV 格式分机明细 ('username,password')" }
                    },
                    "required": ["content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_import_gateways",
                "description": "智能批量导入/更新中继网关节点。对于杂乱的输入文本，大模型先自动提取整理为标准 CSV ('id,name,ip_address,port')，再调用本工具下发绑定。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "由 AI 整理提取出的标准 CSV 格式网关明细 ('id,name,ip_address,port')" }
                    },
                    "required": ["content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_import_routes",
                "description": "智能批量导入前缀选路路由规则。大模型先自动将杂乱输入清洗整理为标准 CSV ('id,prefix,gateway_id,priority')，再调用本工具下发并实时重载选路引擎。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "由 AI 整理提取出的标准 CSV 格式路由明细 ('id,prefix,gateway_id,priority')" }
                    },
                    "required": ["content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_import_rates",
                "description": "智能批量导入/更新资费费率表。大模型先自动将杂乱输入清洗整理为标准 CSV ('prefix,rate_per_minute')，再调用本工具下发。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "由 AI 整理提取出的标准 CSV 格式资费明细 ('prefix,rate_per_minute')" }
                    },
                    "required": ["content"]
                }
            }
        }
    ])
}
