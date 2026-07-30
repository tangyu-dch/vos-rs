# VOS-RS 中继路由与 FreeSWITCH 落地对接测试指南

本指南包含：
1. **中继与路由功能的完整测试步骤与验证方法**（包含 Web 前端测试与 SIP 抓包/命令行测试）
2. **如何将 FreeSWITCH 配置为落地网关 (Egress Trunk) 并验证 VOS-RS 呼出通话**

---

## 一、 中继路由核心功能测试指南

基于我们为您写入的系统测试数据（对接账户、接入中继、落地中继、落地分组、路由策略、号码池组、真实号码），按照以下步骤即可完成全流程验证。

### 1. 前端页面功能与关联验证

#### 测试 1.1：路由仿真测试（含号码池抽号与网关匹配）
- **操作步骤**：
  1. 登录 Web 管理后台，进入 **【中继路由】➔ 【路由策略 (`/routes`)】** 页面。
  2. 点击顶部操作栏的 **【路由仿真】** 按钮，弹出仿真测试弹窗。
  3. **场景 A (只测被叫选路)**：输入目标号码 `13800001001`，不填接入中继。
     - **预期结果**：匹配到前缀 `138` 规则，页面清晰展示指向落地网关 `egress-cmcc-bj` (10.0.1.10:5060)，成本 `¥0.03/分`。
  4. **场景 B (测中继+号码池抽号)**：
     - 在“目标号码”输入 `13800001001`。
     - 在“接入中继”输入 `access-alpha`，点击“执行仿真”。
     - **预期结果**：顶部亮蓝色卡片显示抽号结果 `主叫号码: 13800001001` (或 `13800001002`/`13800001003`)，且显示绑定号码池 `pool-alpha-mobile`。

#### 测试 1.2：落地分组与成员配置 (Egress Groups)
- **操作步骤**：
  1. 进入 **【中继路由】➔ 【落地分组 (`/egress-groups`)】** 页面。
  2. 列表中点击分组 `group-cmcc` 关联列的 **“配置成员 (分支图标)”** 按钮。
  3. 页面将单页无刷新跳转至 `#/egress-groups/group-cmcc` 细节配置页。
  4. **预期结果**：可以看到该组包含了 `egress-cmcc-bj` (优先级10，主用) 和 `egress-cmcc-sh` (优先级20，备用故障切换)。

#### 测试 1.3：呼入目标映射 (DID Inbound)
- **操作步骤**：
  1. 进入 **【中继路由】➔ 【呼入目标 (`/did-destinations`)】** 页面。
  2. 点击**新建目标**：
     - **DID 号码**：`13800001001`
     - **目标类型**：选择 `分机号码 (extension)` 或 `语音导航 (ivr)`
     - **目标标识**：填写分机号 `1001` 或 IVR 标识 `main-sales`
  3. 点击保存，确认已成功持久化。

---

### 2. 自动化集成测试 (SIPp 脚本测试)

如果想要进行无真实软电话的信令级压测与防盗打/主叫改写断言测试，系统自带了完整的 SIPp 脚本套件：

```bash
# 进入项目根目录，运行完整的业务信令断言测试
tools/sipp/run_business_flows.sh all
```

该脚本会自动测试以下 6 大场景：
- `passthrough`：接入透传主叫测试
- `fixed`：固定号码改写主叫测试
- `pool`：号码池动态轮询与首成员容量满退避测试
- `extension-out`：分机外呼主叫改写测试
- `extension-in`：DID 号码归属落地中继呼入投递分机测试
- `owner-failure`：中继故障防冒用安全拦截测试

---

## 二、 使用 FreeSWITCH 作为落地网关 (Egress Trunk) 验证呼出

是的，**完全可以使用 FreeSWITCH 作为落地网关**。VOS-RS 在接收到客户端/接入侧的 SIP 呼叫后，经过选路引擎挑选，会作为一个标准 SIP B2BUA 将 `INVITE` 送往 FreeSWITCH 的 SIP 端口（默认 5060 或 5080）。

### 1. 架构拓扑

```
[ SIP 客户端 / 软电话 / MicroSIP ]
             │  (INVITE 呼出)
             ▼
┌────────────────────────────────────────┐
│   VOS-RS VoIP 软交换平台                │
│   - SIP Edge 接入鉴权 (IP/Digest)       │
│   - 号码池主叫改写 (From / PAI)         │
│   - 路由引擎挑选落地中继                 │
└────────────────────────────────────────┘
             │  (出站 INVITE)
             ▼
┌────────────────────────────────────────┐
│   FreeSWITCH 落地网关 (Egress Trunk)    │
│   - IP: 192.168.x.x  Port: 5060/5080   │
│   - 运行 9196 环回测试音 / 转发 PSTN    │
└────────────────────────────────────────┘
```

---

### 2. FreeSWITCH 侧的配置 (允许来自 VOS-RS 的呼叫)

在 FreeSWITCH 服务器上，需确保默认的 `external` 或 `internal` 饮用配置文件（Profile）允许来自 VOS-RS IP 地址的呼入：

1. **配置 ACL 白名单 (`conf/autoload_configs/acl.conf.xml`)**：
   在 `domains` ACL 中添加 VOS-RS 服务器的 IP：
   ```xml
   <node type="allow" cidr="192.168.1.50/32"/> <!-- 替换为 VOS-RS 实际 IP -->
   ```

2. **配置入站拨号计划 Dialplan (`conf/dialplan/public.xml` 或 `default.xml`)**：
   配置一条响应任意被叫并播放音乐/Echo 回音测试的规则（便于验证通话打通）：
   ```xml
   <extension name="vos_rs_egress_test">
     <condition field="destination_number" expression="^(.*)$">
       <!-- 应答呼叫 -->
       <action application="answer"/>
       <!-- 播放等待音或 Echo 音频回环测试 -->
       <action application="echo"/>
     </condition>
   </extension>
   ```

3. **在 FreeSWITCH 控制台刷新配置**：
   ```bash
   fs_cli -x "reloadxml"
   ```

---

### 3. VOS-RS 侧的对接配置

#### 步骤 3.1：新建落地中继指向 FreeSWITCH
进入 VOS-RS 后台 **【中继路由】➔ 【落地中继 (`/trunks/egress`)】** 页面，点击 **新建落地中继**：
- **中继标识**：`freeswitch-egress`
- **对端主机地址**：`192.168.1.100`（填写您的 FreeSWITCH 服务器 IP）
- **信令端口**：`5060`（或 FreeSWITCH 的 `5080` 端口）
- **传输协议**：`udp`
- **容量上限**：`200`
- **启用状态**：`开启`

#### 步骤 3.2：配置路由策略送往 FreeSWITCH
进入 **【中继路由】➔ 【路由策略 (`/routes`)】** 页面，新建或编辑一条路由规则：
- **路由标识**：`route-to-freeswitch`
- **匹配前缀**：`9`（或者留空匹配所有号码）
- **目标网关**：选择刚才创建的 `freeswitch-egress`
- **优先级别**：`10`
- **呼叫成本**：`0.02`

---

### 4. 发起实际呼出通话验证

1. **软电话注册/接入**：
   将软电话（如 MicroSIP / Zoiper）注册到 VOS-RS 分机（如 `1001`，密码 `admin123`），或使用已被添加为 VOS-RS `接入中继` IP 白名单的软电话。
2. **拨打电话**：
   在软电话上拨打被叫号码 `9196` 或 `98888`。
3. **观察执行结果**：
   - **VOS-RS 控制台日志 / SipFlow 抓包**：
     可以看到 VOS-RS 接收到了软电话的 `INVITE`，并迅速匹配到前缀 `9` 路由策略，向 FreeSWITCH (`192.168.1.100:5060`) 发出了 `INVITE sip:9196@192.168.1.100:5060`。
   - **FreeSWITCH 控制台 (`fs_cli`)**：
     可以看到来自 VOS-RS 的 `INVITE` 成功进入并命中了 `vos_rs_egress_test` 规则，显示 `EXECUTE echo`。
   - **通话接通**：
     软电话听到回音或测试语音，呼出流程验证成功！
