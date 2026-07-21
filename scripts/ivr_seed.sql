-- IVR 测试数据 (重新生成)
-- 覆盖典型业务场景: 总机导航 / VIP 服务 / 售后支持 / 节假日 / 投诉建议
-- 每个 IVR 菜单同时插入:
--   1. 基础字段 (ivr_menus)
--   2. 按键映射 (ivr_actions) - 兼容旧 DTMF 模式
--   3. 可视化拓扑 (nodes/edges JSONB) - 画布编辑模式
-- 这样前端「拓扑编排」Modal 打开时画布立即显示完整节点 + 连线, 无需 fallback

BEGIN;

-- 清理旧数据 (按外键顺序)
DELETE FROM ivr_actions;
DELETE FROM ivr_menus;

-- ========================================
-- 1. 企业总机导航 (main-ivr)
-- 6 个按键分支: 0前台 / 1销售 / 2客服 / 3技术 / 9售后 / *留言
-- ========================================
INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) VALUES
  ('main-ivr',
   '企业总机导航',
   '企业总机欢迎导航菜单, 6 个按键分支',
   '4001010101',
   'welcome_main.wav',
   15,
   TRUE,
   '[
     {"id":"main-start","type":"start","title":"呼入入口","description":"DID 4001010101","position":{"x":80,"y":280},"config":{"did":"4001010101","welcome_prompt":"welcome_main.wav"}},
     {"id":"main-menu","type":"menu","title":"多级菜单","description":"主菜单分支","position":{"x":380,"y":280},"config":{"prompt":"欢迎致电,请按键选择服务","options":[{"key":"0","label":"前台"},{"key":"1","label":"销售"},{"key":"2","label":"客服"},{"key":"3","label":"技术"},{"key":"9","label":"售后"},{"key":"*","label":"留言"}]}},
     {"id":"main-q-reception","type":"transfer_queue","title":"前台队列","description":"reception_queue","position":{"x":760,"y":60},"config":{"queue_id":"reception_queue","priority":5,"skill":"general","timeout_secs":60}},
     {"id":"main-ext-sales","type":"transfer_ext","title":"销售分机","description":"8001","position":{"x":760,"y":160},"config":{"extension":"8001","timeout_secs":30}},
     {"id":"main-ext-service","type":"transfer_ext","title":"客服分机","description":"8002","position":{"x":760,"y":260},"config":{"extension":"8002","timeout_secs":30}},
     {"id":"main-q-tech","type":"transfer_queue","title":"技术队列","description":"support_queue","position":{"x":760,"y":360},"config":{"queue_id":"support_queue","priority":5,"skill":"tech","timeout_secs":60}},
     {"id":"main-ivr-aftersales","type":"transfer_pstn","title":"转接售后 IVR","description":"after-sales-ivr","position":{"x":760,"y":460},"config":{"trunk_id":"internal","target_number":"after-sales-ivr","caller_id":"auto"}},
     {"id":"main-voicemail","type":"voicemail","title":"留言信箱","description":"vm@vos-rs.local","position":{"x":760,"y":560},"config":{"max_duration_secs":60,"prompt":"请在滴声后留言"}},
     {"id":"main-hangup","type":"hangup","title":"挂断","description":"超时挂断","position":{"x":1080,"y":280},"config":{"reason":"timeout","playbye":true}}
   ]'::jsonb,
   '[
     {"id":"main-e1","source":"main-start","target":"main-menu","source_port":"out","label":"进入"},
     {"id":"main-e2","source":"main-menu","target":"main-q-reception","source_port":"key-0","label":"按 0"},
     {"id":"main-e3","source":"main-menu","target":"main-ext-sales","source_port":"key-1","label":"按 1"},
     {"id":"main-e4","source":"main-menu","target":"main-ext-service","source_port":"key-2","label":"按 2"},
     {"id":"main-e5","source":"main-menu","target":"main-q-tech","source_port":"key-3","label":"按 3"},
     {"id":"main-e6","source":"main-menu","target":"main-ivr-aftersales","source_port":"key-9","label":"按 9"},
     {"id":"main-e7","source":"main-menu","target":"main-voicemail","source_port":"key-*","label":"按 *"},
     {"id":"main-e8","source":"main-menu","target":"main-hangup","source_port":"default","label":"超时"}
   ]'::jsonb
  )
ON CONFLICT (id) DO UPDATE SET
  name = EXCLUDED.name,
  description = EXCLUDED.description,
  did = EXCLUDED.did,
  welcome_prompt = EXCLUDED.welcome_prompt,
  timeout_secs = EXCLUDED.timeout_secs,
  enabled = EXCLUDED.enabled,
  nodes = EXCLUDED.nodes,
  edges = EXCLUDED.edges;

INSERT INTO ivr_actions (ivr_id, dtmf_key, action_type, action_target, waiting_prompt, webhook_method) VALUES
  ('main-ivr', '0', 'queue',       'reception_queue',     '转接前台.wav',     NULL),
  ('main-ivr', '1', 'extension',   '8001',                '转接销售.wav',     NULL),
  ('main-ivr', '2', 'extension',   '8002',                '转接客服.wav',     NULL),
  ('main-ivr', '3', 'queue',       'support_queue',       '转接技术.wav',     NULL),
  ('main-ivr', '9', 'ivr',         'after-sales-ivr',     '转接售后.wav',     NULL),
  ('main-ivr', '*', 'voicemail',   'vm@vos-rs.local',     '留言信箱.wav',     NULL)
ON CONFLICT (ivr_id, dtmf_key) DO UPDATE SET
  action_type = EXCLUDED.action_type,
  action_target = EXCLUDED.action_target,
  waiting_prompt = EXCLUDED.waiting_prompt,
  webhook_method = EXCLUDED.webhook_method;

-- ========================================
-- 2. VIP 客户专属服务 (vip-ivr)
-- 5 个按键分支: 1顾问 / 2财富管理 / 3webhook / 4订单播报 / 0接待
-- ========================================
INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) VALUES
  ('vip-ivr',
   'VIP 客户专属服务',
   'VIP 客户专属服务菜单, 优先接入专属顾问',
   '4001010202',
   'welcome_vip.wav',
   20,
   TRUE,
   '[
     {"id":"vip-start","type":"start","title":"呼入入口","description":"DID 4001010202 (VIP)","position":{"x":80,"y":280},"config":{"did":"4001010202","welcome_prompt":"welcome_vip.wav"}},
     {"id":"vip-menu","type":"menu","title":"VIP 主菜单","description":"VIP 专属分支","position":{"x":380,"y":280},"config":{"prompt":"尊敬的 VIP 客户,请按键选择服务","options":[{"key":"1","label":"专属顾问"},{"key":"2","label":"财富管理"},{"key":"3","label":"智能调度"},{"key":"4","label":"订单查询"},{"key":"0","label":"VIP 接待"}]}},
     {"id":"vip-ext-advisor","type":"transfer_ext","title":"专属顾问","description":"9001","position":{"x":760,"y":100},"config":{"extension":"9001","timeout_secs":30}},
     {"id":"vip-q-finance","type":"transfer_queue","title":"财富管理队列","description":"vip_finance_queue","position":{"x":760,"y":220},"config":{"queue_id":"vip_finance_queue","priority":3,"skill":"finance","timeout_secs":90}},
     {"id":"vip-webhook","type":"http_webhook","title":"CRM 智能调度","description":"POST /ivr/vip/dispatch","position":{"x":760,"y":340},"config":{"url":"https://api.crm.example.com/ivr/vip/dispatch","method":"POST","headers":{"Content-Type":"application/json"},"timeout_secs":5}},
     {"id":"vip-tts-order","type":"tts","title":"订单播报","description":"订单顺丰发货,明日送达","position":{"x":760,"y":460},"config":{"text":"尊敬的 VIP 客户,您的订单已顺丰发货,预计明日送达","voice":"female-zh-CN","speed":1.0}},
     {"id":"vip-q-reception","type":"transfer_queue","title":"VIP 接待队列","description":"vip_reception_queue","position":{"x":760,"y":580},"config":{"queue_id":"vip_reception_queue","priority":1,"skill":"vip","timeout_secs":120}},
     {"id":"vip-hangup","type":"hangup","title":"挂断","description":"超时挂断","position":{"x":1080,"y":280},"config":{"reason":"timeout","playbye":true}}
   ]'::jsonb,
   '[
     {"id":"vip-e1","source":"vip-start","target":"vip-menu","source_port":"out","label":"进入"},
     {"id":"vip-e2","source":"vip-menu","target":"vip-ext-advisor","source_port":"key-1","label":"按 1"},
     {"id":"vip-e3","source":"vip-menu","target":"vip-q-finance","source_port":"key-2","label":"按 2"},
     {"id":"vip-e4","source":"vip-menu","target":"vip-webhook","source_port":"key-3","label":"按 3"},
     {"id":"vip-e5","source":"vip-menu","target":"vip-tts-order","source_port":"key-4","label":"按 4"},
     {"id":"vip-e6","source":"vip-menu","target":"vip-q-reception","source_port":"key-0","label":"按 0"},
     {"id":"vip-e7","source":"vip-menu","target":"vip-hangup","source_port":"default","label":"超时"}
   ]'::jsonb
  )
ON CONFLICT (id) DO UPDATE SET
  name = EXCLUDED.name,
  description = EXCLUDED.description,
  did = EXCLUDED.did,
  welcome_prompt = EXCLUDED.welcome_prompt,
  timeout_secs = EXCLUDED.timeout_secs,
  enabled = EXCLUDED.enabled,
  nodes = EXCLUDED.nodes,
  edges = EXCLUDED.edges;

INSERT INTO ivr_actions (ivr_id, dtmf_key, action_type, action_target, waiting_prompt, webhook_method) VALUES
  ('vip-ivr', '1', 'extension',   '9001',                                       '转接专属顾问.wav', NULL),
  ('vip-ivr', '2', 'queue',       'vip_finance_queue',                          '转接财富管理.wav', NULL),
  ('vip-ivr', '3', 'webhook',     'https://api.crm.example.com/ivr/vip/dispatch','please_wait.wav',  'POST'),
  ('vip-ivr', '4', 'say',         '尊敬的 VIP 客户,您的订单已顺丰发货,预计明日送达', 'prompt_notice.wav', NULL),
  ('vip-ivr', '0', 'queue',       'vip_reception_queue',                        '转接 VIP 接待.wav', NULL)
ON CONFLICT (ivr_id, dtmf_key) DO UPDATE SET
  action_type = EXCLUDED.action_type,
  action_target = EXCLUDED.action_target,
  waiting_prompt = EXCLUDED.waiting_prompt,
  webhook_method = EXCLUDED.webhook_method;

-- ========================================
-- 3. 售后服务菜单 (after-sales-ivr)
-- 5 个按键分支: 1报修 / 2退换货 / 3专员 / 4工单 / 0留言
-- ========================================
INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) VALUES
  ('after-sales-ivr',
   '售后服务菜单',
   '售后设备报修、退换货、工单查询服务',
   '4001010303',
   'welcome_after_sales.wav',
   18,
   TRUE,
   '[
     {"id":"as-start","type":"start","title":"呼入入口","description":"DID 4001010303","position":{"x":80,"y":280},"config":{"did":"4001010303","welcome_prompt":"welcome_after_sales.wav"}},
     {"id":"as-menu","type":"menu","title":"售后主菜单","description":"售后分支","position":{"x":380,"y":280},"config":{"prompt":"欢迎进入售后服务,请按键选择","options":[{"key":"1","label":"设备报修"},{"key":"2","label":"退换货"},{"key":"3","label":"转专员"},{"key":"4","label":"工单查询"},{"key":"0","label":"留言"}]}},
     {"id":"as-q-repair","type":"transfer_queue","title":"报修队列","description":"repair_queue","position":{"x":760,"y":100},"config":{"queue_id":"repair_queue","priority":5,"skill":"repair","timeout_secs":60}},
     {"id":"as-q-return","type":"transfer_queue","title":"退换货队列","description":"return_queue","position":{"x":760,"y":220},"config":{"queue_id":"return_queue","priority":5,"skill":"return","timeout_secs":60}},
     {"id":"as-ext-agent","type":"transfer_ext","title":"售后专员","description":"8003","position":{"x":760,"y":340},"config":{"extension":"8003","timeout_secs":30}},
     {"id":"as-webhook-ticket","type":"http_webhook","title":"工单查询","description":"GET /api/ticket","position":{"x":760,"y":460},"config":{"url":"https://crm.example.com/api/ticket","method":"GET","headers":{"Content-Type":"application/json"},"timeout_secs":5}},
     {"id":"as-voicemail","type":"voicemail","title":"留言信箱","description":"vm_after_sales@vos-rs.local","position":{"x":760,"y":580},"config":{"max_duration_secs":90,"prompt":"请在滴声后留言"}},
     {"id":"as-hangup","type":"hangup","title":"挂断","description":"超时挂断","position":{"x":1080,"y":280},"config":{"reason":"timeout","playbye":true}}
   ]'::jsonb,
   '[
     {"id":"as-e1","source":"as-start","target":"as-menu","source_port":"out","label":"进入"},
     {"id":"as-e2","source":"as-menu","target":"as-q-repair","source_port":"key-1","label":"按 1"},
     {"id":"as-e3","source":"as-menu","target":"as-q-return","source_port":"key-2","label":"按 2"},
     {"id":"as-e4","source":"as-menu","target":"as-ext-agent","source_port":"key-3","label":"按 3"},
     {"id":"as-e5","source":"as-menu","target":"as-webhook-ticket","source_port":"key-4","label":"按 4"},
     {"id":"as-e6","source":"as-menu","target":"as-voicemail","source_port":"key-0","label":"按 0"},
     {"id":"as-e7","source":"as-menu","target":"as-hangup","source_port":"default","label":"超时"}
   ]'::jsonb
  )
ON CONFLICT (id) DO UPDATE SET
  name = EXCLUDED.name,
  description = EXCLUDED.description,
  did = EXCLUDED.did,
  welcome_prompt = EXCLUDED.welcome_prompt,
  timeout_secs = EXCLUDED.timeout_secs,
  enabled = EXCLUDED.enabled,
  nodes = EXCLUDED.nodes,
  edges = EXCLUDED.edges;

INSERT INTO ivr_actions (ivr_id, dtmf_key, action_type, action_target, waiting_prompt, webhook_method) VALUES
  ('after-sales-ivr', '1', 'queue',       'repair_queue',         '设备报修.wav',     NULL),
  ('after-sales-ivr', '2', 'queue',       'return_queue',         '退换货服务.wav',   NULL),
  ('after-sales-ivr', '3', 'extension',   '8003',                 '转接售后专员.wav', NULL),
  ('after-sales-ivr', '4', 'webhook',     'https://crm.example.com/api/ticket', '工单查询.wav', 'GET'),
  ('after-sales-ivr', '0', 'voicemail',   'vm_after_sales@vos-rs.local', '留言信箱.wav', NULL)
ON CONFLICT (ivr_id, dtmf_key) DO UPDATE SET
  action_type = EXCLUDED.action_type,
  action_target = EXCLUDED.action_target,
  waiting_prompt = EXCLUDED.waiting_prompt,
  webhook_method = EXCLUDED.webhook_method;

-- ========================================
-- 4. 节假日特殊菜单 (holiday-ivr)
-- 4 个按键分支: 1营业时间 / 2值班销售 / 3值班客服 / 0留言
-- ========================================
INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) VALUES
  ('holiday-ivr',
   '节假日特殊菜单',
   '节假日值班服务菜单, 营业时间播报 + 值班转接',
   '4001010404',
   'welcome_holiday.wav',
   12,
   TRUE,
   '[
     {"id":"hd-start","type":"start","title":"呼入入口","description":"DID 4001010404","position":{"x":80,"y":280},"config":{"did":"4001010404","welcome_prompt":"welcome_holiday.wav"}},
     {"id":"hd-menu","type":"menu","title":"节假日主菜单","description":"节假日分支","position":{"x":380,"y":280},"config":{"prompt":"节假日专属服务,请按键选择","options":[{"key":"1","label":"营业时间"},{"key":"2","label":"值班销售"},{"key":"3","label":"值班客服"},{"key":"0","label":"留言"}]}},
     {"id":"hd-tts-hours","type":"tts","title":"营业时间播报","description":"9:00-18:00 营业","position":{"x":760,"y":160},"config":{"text":"尊敬的客户,我司节假日正常营业,工作时间为 9:00-18:00","voice":"female-zh-CN","speed":1.0}},
     {"id":"hd-ext-sales","type":"transfer_ext","title":"值班销售","description":"8001","position":{"x":760,"y":300},"config":{"extension":"8001","timeout_secs":30}},
     {"id":"hd-q-support","type":"transfer_queue","title":"值班客服队列","description":"holiday_support_queue","position":{"x":760,"y":440},"config":{"queue_id":"holiday_support_queue","priority":5,"skill":"holiday","timeout_secs":60}},
     {"id":"hd-voicemail","type":"voicemail","title":"留言信箱","description":"vm_holiday@vos-rs.local","position":{"x":760,"y":580},"config":{"max_duration_secs":60,"prompt":"请在滴声后留言"}},
     {"id":"hd-hangup","type":"hangup","title":"挂断","description":"超时挂断","position":{"x":1080,"y":280},"config":{"reason":"timeout","playbye":true}}
   ]'::jsonb,
   '[
     {"id":"hd-e1","source":"hd-start","target":"hd-menu","source_port":"out","label":"进入"},
     {"id":"hd-e2","source":"hd-menu","target":"hd-tts-hours","source_port":"key-1","label":"按 1"},
     {"id":"hd-e3","source":"hd-menu","target":"hd-ext-sales","source_port":"key-2","label":"按 2"},
     {"id":"hd-e4","source":"hd-menu","target":"hd-q-support","source_port":"key-3","label":"按 3"},
     {"id":"hd-e5","source":"hd-menu","target":"hd-voicemail","source_port":"key-0","label":"按 0"},
     {"id":"hd-e6","source":"hd-menu","target":"hd-hangup","source_port":"default","label":"超时"}
   ]'::jsonb
  )
ON CONFLICT (id) DO UPDATE SET
  name = EXCLUDED.name,
  description = EXCLUDED.description,
  did = EXCLUDED.did,
  welcome_prompt = EXCLUDED.welcome_prompt,
  timeout_secs = EXCLUDED.timeout_secs,
  enabled = EXCLUDED.enabled,
  nodes = EXCLUDED.nodes,
  edges = EXCLUDED.edges;

INSERT INTO ivr_actions (ivr_id, dtmf_key, action_type, action_target, waiting_prompt, webhook_method) VALUES
  ('holiday-ivr', '1', 'say',         '尊敬的客户,我司节假日正常营业,工作时间为 9:00-18:00', NULL, NULL),
  ('holiday-ivr', '2', 'extension',   '8001',          '转接值班销售.wav', NULL),
  ('holiday-ivr', '3', 'queue',       'holiday_support_queue', '转接值班客服.wav', NULL),
  ('holiday-ivr', '0', 'voicemail',   'vm_holiday@vos-rs.local', '留言信箱.wav', NULL)
ON CONFLICT (ivr_id, dtmf_key) DO UPDATE SET
  action_type = EXCLUDED.action_type,
  action_target = EXCLUDED.action_target,
  waiting_prompt = EXCLUDED.waiting_prompt,
  webhook_method = EXCLUDED.webhook_method;

-- ========================================
-- 5. 投诉建议菜单 (feedback-ivr)
-- 5 个按键分支: 1投诉 / 2建议 / 3主管 / 4提交反馈 / 0留言
-- ========================================
INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) VALUES
  ('feedback-ivr',
   '投诉建议菜单',
   '投诉受理、建议收集、反馈提交服务',
   '4001010505',
   'welcome_feedback.wav',
   25,
   TRUE,
   '[
     {"id":"fb-start","type":"start","title":"呼入入口","description":"DID 4001010505","position":{"x":80,"y":280},"config":{"did":"4001010505","welcome_prompt":"welcome_feedback.wav"}},
     {"id":"fb-menu","type":"menu","title":"投诉建议主菜单","description":"投诉建议分支","position":{"x":380,"y":280},"config":{"prompt":"欢迎进入投诉建议服务,请按键选择","options":[{"key":"1","label":"投诉"},{"key":"2","label":"建议"},{"key":"3","label":"主管"},{"key":"4","label":"提交反馈"},{"key":"0","label":"留言"}]}},
     {"id":"fb-q-complaint","type":"transfer_queue","title":"投诉受理队列","description":"complaint_queue","position":{"x":760,"y":100},"config":{"queue_id":"complaint_queue","priority":3,"skill":"complaint","timeout_secs":120}},
     {"id":"fb-q-suggestion","type":"transfer_queue","title":"建议受理队列","description":"suggestion_queue","position":{"x":760,"y":220},"config":{"queue_id":"suggestion_queue","priority":5,"skill":"suggestion","timeout_secs":60}},
     {"id":"fb-ext-manager","type":"transfer_ext","title":"客服主管","description":"8004","position":{"x":760,"y":340},"config":{"extension":"8004","timeout_secs":30}},
     {"id":"fb-webhook-submit","type":"http_webhook","title":"反馈提交","description":"POST /api/feedback","position":{"x":760,"y":460},"config":{"url":"https://crm.example.com/api/feedback","method":"POST","headers":{"Content-Type":"application/json"},"timeout_secs":5}},
     {"id":"fb-voicemail","type":"voicemail","title":"留言信箱","description":"vm_feedback@vos-rs.local","position":{"x":760,"y":580},"config":{"max_duration_secs":120,"prompt":"请在滴声后留言"}},
     {"id":"fb-hangup","type":"hangup","title":"挂断","description":"超时挂断","position":{"x":1080,"y":280},"config":{"reason":"timeout","playbye":true}}
   ]'::jsonb,
   '[
     {"id":"fb-e1","source":"fb-start","target":"fb-menu","source_port":"out","label":"进入"},
     {"id":"fb-e2","source":"fb-menu","target":"fb-q-complaint","source_port":"key-1","label":"按 1"},
     {"id":"fb-e3","source":"fb-menu","target":"fb-q-suggestion","source_port":"key-2","label":"按 2"},
     {"id":"fb-e4","source":"fb-menu","target":"fb-ext-manager","source_port":"key-3","label":"按 3"},
     {"id":"fb-e5","source":"fb-menu","target":"fb-webhook-submit","source_port":"key-4","label":"按 4"},
     {"id":"fb-e6","source":"fb-menu","target":"fb-voicemail","source_port":"key-0","label":"按 0"},
     {"id":"fb-e7","source":"fb-menu","target":"fb-hangup","source_port":"default","label":"超时"}
   ]'::jsonb
  )
ON CONFLICT (id) DO UPDATE SET
  name = EXCLUDED.name,
  description = EXCLUDED.description,
  did = EXCLUDED.did,
  welcome_prompt = EXCLUDED.welcome_prompt,
  timeout_secs = EXCLUDED.timeout_secs,
  enabled = EXCLUDED.enabled,
  nodes = EXCLUDED.nodes,
  edges = EXCLUDED.edges;

INSERT INTO ivr_actions (ivr_id, dtmf_key, action_type, action_target, waiting_prompt, webhook_method) VALUES
  ('feedback-ivr', '1', 'queue',       'complaint_queue',  '投诉受理.wav',    NULL),
  ('feedback-ivr', '2', 'queue',       'suggestion_queue', '建议受理.wav',    NULL),
  ('feedback-ivr', '3', 'extension',   '8004',             '转接客服主管.wav', NULL),
  ('feedback-ivr', '4', 'webhook',     'https://crm.example.com/api/feedback', '提交反馈.wav', 'POST'),
  ('feedback-ivr', '0', 'voicemail',   'vm_feedback@vos-rs.local', '语音留言.wav', NULL)
ON CONFLICT (ivr_id, dtmf_key) DO UPDATE SET
  action_type = EXCLUDED.action_type,
  action_target = EXCLUDED.action_target,
  waiting_prompt = EXCLUDED.waiting_prompt,
  webhook_method = EXCLUDED.webhook_method;

COMMIT;

-- 验证数据
SELECT '=== IVR 菜单清单 (含拓扑) ===' AS info;
SELECT id, name, did, enabled,
       jsonb_array_length(nodes) AS node_count,
       jsonb_array_length(edges) AS edge_count
  FROM ivr_menus ORDER BY id;

SELECT '=== IVR 按键映射统计 ===' AS info;
SELECT ivr_id, COUNT(*) AS mapping_count
  FROM ivr_actions
 GROUP BY ivr_id
 ORDER BY ivr_id;

SELECT '=== 总计 ===' AS info;
SELECT
  (SELECT COUNT(*) FROM ivr_menus) AS menus,
  (SELECT COUNT(*) FROM ivr_actions) AS mappings,
  (SELECT SUM(jsonb_array_length(nodes)) FROM ivr_menus) AS total_nodes,
  (SELECT SUM(jsonb_array_length(edges)) FROM ivr_menus) AS total_edges;
