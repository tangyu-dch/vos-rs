// 路由节点配置表单 - 按节点类型设计专属表单（替代 JSON 显示）
// 与 IVR 节点表单设计风格统一

import { Input, Textarea, Select, SelectItem, Chip } from '@heroui/react';
import { Plus, X } from 'lucide-react';
import type { RouteNodeType } from './route-types';

// 通用字段包装
function Field({
  label,
  children,
  hint,
}: {
  label: string;
  children: React.ReactNode;
  hint?: string;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-xs font-semibold text-foreground">{label}</label>
      {children}
      {hint && <span className="text-[10px] text-default-400">{hint}</span>}
    </div>
  );
}

interface RouteNodeFormProps {
  type: RouteNodeType;
  config: Record<string, unknown>;
  onChange: (config: Record<string, unknown>) => void;
}

function setField<T extends Record<string, unknown>>(
  config: T,
  key: string,
  value: unknown,
): T {
  return { ...config, [key]: value };
}

export function RouteNodeConfigForm({ type, config, onChange }: RouteNodeFormProps) {
  const update = (key: string, value: unknown) => onChange(setField(config, key, value));

  switch (type) {
    case 'inbound':
      return (
        <div className="flex flex-col gap-3">
          <Field label="来源类型">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.source_type ?? 'did')]}
              onChange={(e) => update('source_type', e.target.value)}
            >
              <SelectItem key="did">DID 号码</SelectItem>
              <SelectItem key="trunk">中继入呼</SelectItem>
              <SelectItem key="any">任意来源</SelectItem>
            </Select>
          </Field>
          {config.source_type !== 'any' && (
            <Field label={config.source_type === 'did' ? 'DID 号码' : '中继 ID'} hint="留空表示匹配全部">
              <Input
                variant="bordered"
                size="sm"
                placeholder={config.source_type === 'did' ? '4008009000' : 'gw-telecom'}
                value={String(config.did ?? config.trunk_id ?? '')}
                onValueChange={(v) => update(config.source_type === 'did' ? 'did' : 'trunk_id', v)}
              />
            </Field>
          )}
        </div>
      );

    case 'prefix_match': {
      const prefixes = Array.isArray(config.prefixes)
        ? (config.prefixes as Array<{ prefix: string; label: string }>)
        : [];
      return (
        <div className="flex flex-col gap-3">
          <Field label="前缀规则" hint="每个前缀会生成一个出口端口">
            <div className="flex flex-col gap-2">
              {prefixes.map((item, idx) => (
                <div key={idx} className="flex items-center gap-2">
                  <Input
                    variant="bordered"
                    size="sm"
                    className="w-24"
                    placeholder="86"
                    value={item.prefix}
                    onValueChange={(v) => {
                      const next = [...prefixes];
                      next[idx] = { ...item, prefix: v };
                      update('prefixes', next);
                    }}
                  />
                  <Input
                    variant="bordered"
                    size="sm"
                    className="flex-1"
                    placeholder="标签 (如: 中国大陆)"
                    value={item.label}
                    onValueChange={(v) => {
                      const next = [...prefixes];
                      next[idx] = { ...item, label: v };
                      update('prefixes', next);
                    }}
                  />
                  <button
                    type="button"
                    className="shrink-0 w-7 h-7 rounded-md flex items-center justify-center text-danger hover:bg-danger/10"
                    onClick={() => update('prefixes', prefixes.filter((_, i) => i !== idx))}
                    aria-label="删除前缀"
                  >
                    <X className="w-3.5 h-3.5" />
                  </button>
                </div>
              ))}
              <button
                type="button"
                className="flex items-center gap-1.5 text-xs text-primary font-semibold self-start"
                onClick={() => update('prefixes', [...prefixes, { prefix: '', label: `前缀 ${prefixes.length + 1}` }])}
              >
                <Plus className="w-3.5 h-3.5" />
                添加前缀
              </button>
            </div>
          </Field>
        </div>
      );
    }

    case 'time_filter':
      return (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label="开始时间">
              <Input
                variant="bordered"
                size="sm"
                placeholder="09:00"
                value={String(config.time_start ?? '09:00')}
                onValueChange={(v) => update('time_start', v)}
              />
            </Field>
            <Field label="结束时间">
              <Input
                variant="bordered"
                size="sm"
                placeholder="18:00"
                value={String(config.time_end ?? '18:00')}
                onValueChange={(v) => update('time_end', v)}
              />
            </Field>
          </div>
          <Field label="生效星期" hint="1=周一, 7=周日">
            <div className="flex flex-wrap gap-1.5">
              {[1, 2, 3, 4, 5, 6, 7].map((day) => {
                const weekdays = Array.isArray(config.weekdays) ? (config.weekdays as number[]) : [];
                const isSelected = weekdays.includes(day);
                const dayLabel = ['一', '二', '三', '四', '五', '六', '日'][day - 1];
                return (
                  <Chip
                    key={day}
                    size="sm"
                    variant={isSelected ? 'solid' : 'flat'}
                    color={isSelected ? 'primary' : 'default'}
                    className="cursor-pointer select-none w-8 justify-center"
                    onClick={() => {
                      const next = isSelected
                        ? weekdays.filter((d) => d !== day)
                        : [...weekdays, day].sort();
                      update('weekdays', next);
                    }}
                  >
                    {dayLabel}
                  </Chip>
                );
              })}
            </div>
          </Field>
          <Field label="时区">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.timezone ?? 'Asia/Shanghai')]}
              onChange={(e) => update('timezone', e.target.value)}
            >
              <SelectItem key="Asia/Shanghai">Asia/Shanghai (UTC+8)</SelectItem>
              <SelectItem key="UTC">UTC</SelectItem>
              <SelectItem key="America/New_York">America/New_York (UTC-5)</SelectItem>
              <SelectItem key="Europe/London">Europe/London (UTC+0)</SelectItem>
            </Select>
          </Field>
        </div>
      );

    case 'caller_filter':
      return (
        <div className="flex flex-col gap-3">
          <Field label="过滤模式">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.mode ?? 'whitelist')]}
              onChange={(e) => update('mode', e.target.value)}
            >
              <SelectItem key="whitelist">白名单 (仅允许匹配的号码)</SelectItem>
              <SelectItem key="blacklist">黑名单 (拒绝匹配的号码)</SelectItem>
            </Select>
          </Field>
          <Field label="号码模式" hint="每行一个,支持通配符 (如 138*, 13900138000)">
            <Textarea
              variant="bordered"
              size="sm"
              minRows={4}
              placeholder={'13800138000\n139*\n400*'}
              value={Array.isArray(config.patterns) ? (config.patterns as string[]).join('\n') : ''}
              onValueChange={(v) => update('patterns', v.split('\n').filter((s) => s.trim()))}
            />
          </Field>
        </div>
      );

    case 'lcr':
      return (
        <div className="flex flex-col gap-3">
          <Field label="选路策略">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.strategy ?? 'lowest_cost')]}
              onChange={(e) => update('strategy', e.target.value)}
            >
              <SelectItem key="lowest_cost">最低成本</SelectItem>
              <SelectItem key="highest_quality">最高质量</SelectItem>
              <SelectItem key="round_robin">轮询</SelectItem>
              <SelectItem key="weighted">权重分配</SelectItem>
            </Select>
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="最大跳数">
              <Input
                type="number"
                variant="bordered"
                size="sm"
                min={1}
                max={10}
                value={String(config.max_hops ?? 3)}
                onValueChange={(v) => update('max_hops', Number(v) || 3)}
              />
            </Field>
            <Field label="回退策略">
              <Select
                variant="bordered"
                size="sm"
                selectedKeys={[String(config.fallback ?? 'reject')]}
                onChange={(e) => update('fallback', e.target.value)}
              >
                <SelectItem key="reject">拒绝</SelectItem>
                <SelectItem key="next_rule">尝试下一条规则</SelectItem>
                <SelectItem key="play_busy">播放忙音</SelectItem>
              </Select>
            </Field>
          </div>
        </div>
      );

    case 'gateway_trunk':
      return (
        <div className="flex flex-col gap-3">
          <Field label="中继 ID" hint="填写已配置的 PSTN 网关 ID">
            <Input
              variant="bordered"
              size="sm"
              placeholder="gw-telecom"
              value={String(config.trunk_id ?? '')}
              onValueChange={(v) => update('trunk_id', v)}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="优先级" hint="数值越大优先级越高">
              <Input
                type="number"
                variant="bordered"
                size="sm"
                min={1}
                value={String(config.priority ?? 100)}
                onValueChange={(v) => update('priority', Number(v) || 100)}
              />
            </Field>
            <Field label="权重" hint="同优先级内的分流权重">
              <Input
                type="number"
                variant="bordered"
                size="sm"
                min={1}
                value={String(config.weight ?? 100)}
                onValueChange={(v) => update('weight', Number(v) || 100)}
              />
            </Field>
          </div>
          <Field label="路由成本" hint="每分钟计费成本 (元)">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              step={0.01}
              min={0}
              value={String(config.cost ?? 0)}
              onValueChange={(v) => update('cost', Number(v) || 0)}
            />
          </Field>
        </div>
      );

    case 'ivr_branch':
      return (
        <div className="flex flex-col gap-3">
          <Field label="IVR 流程 ID" hint="转入已创建的 IVR 流程">
            <Input
              variant="bordered"
              size="sm"
              placeholder="ivr-main"
              value={String(config.ivr_id ?? '')}
              onValueChange={(v) => update('ivr_id', v)}
            />
          </Field>
        </div>
      );

    case 'queue_branch':
      return (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label="队列 ID">
              <Input
                variant="bordered"
                size="sm"
                placeholder="queue-support"
                value={String(config.queue_id ?? '')}
                onValueChange={(v) => update('queue_id', v)}
              />
            </Field>
            <Field label="优先级 (1-10)">
              <Input
                type="number"
                variant="bordered"
                size="sm"
                min={1}
                max={10}
                value={String(config.priority ?? 5)}
                onValueChange={(v) => update('priority', Number(v) || 5)}
              />
            </Field>
          </div>
        </div>
      );

    case 'fork':
      return (
        <div className="flex flex-col gap-3">
          <Field label="分支策略">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.strategy ?? 'first_win')]}
              onChange={(e) => update('strategy', e.target.value)}
            >
              <SelectItem key="first_win">首个接通即成功 (First-Win)</SelectItem>
              <SelectItem key="all_wait">等待所有分支结果</SelectItem>
            </Select>
          </Field>
          <Field label="超时 (秒)">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              min={5}
              max={120}
              value={String(config.timeout_secs ?? 30)}
              onValueChange={(v) => update('timeout_secs', Number(v) || 30)}
            />
          </Field>
        </div>
      );

    case 'reject':
      return (
        <div className="flex flex-col gap-3">
          <Field label="拒绝原因">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.reason ?? 'busy')]}
              onChange={(e) => update('reason', e.target.value)}
            >
              <SelectItem key="busy">忙音 (486 Busy Here)</SelectItem>
              <SelectItem key="reject">拒接 (603 Decline)</SelectItem>
              <SelectItem key="not_found">空号 (404 Not Found)</SelectItem>
              <SelectItem key="temp_unavailable">临时不可用 (480)</SelectItem>
            </Select>
          </Field>
          <Field label="SIP 响应码" hint="自定义 SIP 响应码 (可选)">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              min={400}
              max={699}
              value={String(config.sip_code ?? 486)}
              onValueChange={(v) => update('sip_code', Number(v) || 486)}
            />
          </Field>
        </div>
      );

    default:
      return <div className="text-xs text-default-400">此节点类型暂无可配置项</div>;
  }
}
