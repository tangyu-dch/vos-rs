import { Input, Textarea, Switch, Select, SelectItem } from '@heroui/react';
import type { IvrNodeType } from './types';

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

interface NodeFormProps {
  type: IvrNodeType;
  config: Record<string, unknown>;
  onChange: (config: Record<string, unknown>) => void;
}

// 设置单个字段
function setField<T extends Record<string, unknown>>(
  config: T,
  key: string,
  value: unknown
): T {
  return { ...config, [key]: value };
}

export function NodeConfigForm({ type, config, onChange }: NodeFormProps) {
  const update = (key: string, value: unknown) => onChange(setField(config, key, value));

  switch (type) {
    case 'start':
      return (
        <div className="flex flex-col gap-3">
          <Field label="DID 号码" hint="入呼主叫匹配此号码触发流程">
            <Input
              variant="bordered"
              size="sm"
              placeholder="4008009000"
              value={String(config.did ?? '')}
              onValueChange={(v) => update('did', v)}
            />
          </Field>
          <Field label="欢迎语音文件">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.welcome_prompt ?? 'welcome.wav')}
              onValueChange={(v) => update('welcome_prompt', v)}
            />
          </Field>
        </div>
      );

    case 'prompt':
      return (
        <div className="flex flex-col gap-3">
          <Field label="音频文件名">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.audio_file ?? 'prompt.wav')}
              onValueChange={(v) => update('audio_file', v)}
            />
          </Field>
          <Field label="循环次数">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              min={1}
              max={100}
              value={String(config.loop ?? 1)}
              onValueChange={(v) => update('loop', Number(v) || 1)}
            />
          </Field>
          <Field label="允许按键打断">
            <Switch
              size="sm"
              isSelected={Boolean(config.interruptible)}
              onChange={(e) => update('interruptible', e.target.checked)}
            />
          </Field>
        </div>
      );

    case 'tts':
      return (
        <div className="flex flex-col gap-3">
          <Field label="合成文本">
            <Textarea
              variant="bordered"
              size="sm"
              minRows={3}
              value={String(config.text ?? '')}
              onValueChange={(v) => update('text', v)}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="音色">
              <Select
                variant="bordered"
                size="sm"
                selectedKeys={[String(config.voice ?? 'female-zh-CN')]}
                onChange={(e) => update('voice', e.target.value)}
              >
                <SelectItem key="female-zh-CN">女声 (中文)</SelectItem>
                <SelectItem key="male-zh-CN">男声 (中文)</SelectItem>
                <SelectItem key="female-en-US">女声 (英文)</SelectItem>
                <SelectItem key="male-en-US">男声 (英文)</SelectItem>
              </Select>
            </Field>
            <Field label="语速">
              <Input
                type="number"
                variant="bordered"
                size="sm"
                step={0.1}
                min={0.5}
                max={2.0}
                value={String(config.speed ?? 1.0)}
                onValueChange={(v) => update('speed', Number(v) || 1.0)}
              />
            </Field>
          </div>
        </div>
      );

    case 'collect_dtmf':
      return (
        <div className="grid grid-cols-2 gap-3">
          <Field label="最大位数">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              min={1}
              max={16}
              value={String(config.max_digits ?? 4)}
              onValueChange={(v) => update('max_digits', Number(v) || 1)}
            />
          </Field>
          <Field label="超时秒数">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              value={String(config.timeout_secs ?? 5)}
              onValueChange={(v) => update('timeout_secs', Number(v) || 5)}
            />
          </Field>
          <Field label="结束符">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.terminator ?? '#')]}
              onChange={(e) => update('terminator', e.target.value)}
            >
              <SelectItem key="#">#</SelectItem>
              <SelectItem key="*">*</SelectItem>
              <SelectItem key="none">无</SelectItem>
            </Select>
          </Field>
        </div>
      );

    case 'menu': {
      const options = Array.isArray(config.options)
        ? (config.options as Array<{ key: string; label: string }>)
        : [];
      return (
        <div className="flex flex-col gap-3">
          <Field label="提示语">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.prompt ?? '')}
              onValueChange={(v) => update('prompt', v)}
            />
          </Field>
          <Field label="按键选项" hint="每个按键会生成一个出口端口">
            <div className="flex flex-col gap-2">
              {options.map((opt, idx) => (
                <div key={idx} className="flex items-center gap-2">
                  <Input
                    variant="bordered"
                    size="sm"
                    className="w-16"
                    value={opt.key}
                    onValueChange={(v) => {
                      const next = [...options];
                      next[idx] = { ...opt, key: v };
                      update('options', next);
                    }}
                  />
                  <Input
                    variant="bordered"
                    size="sm"
                    value={opt.label}
                    onValueChange={(v) => {
                      const next = [...options];
                      next[idx] = { ...opt, label: v };
                      update('options', next);
                    }}
                  />
                </div>
              ))}
              <div className="flex gap-2">
                <button
                  type="button"
                  className="text-xs text-primary font-semibold"
                  onClick={() => {
                    const next = [...options, { key: String(options.length + 1), label: `选项 ${options.length + 1}` }];
                    update('options', next);
                  }}
                >
                  + 添加选项
                </button>
                {options.length > 0 && (
                  <button
                    type="button"
                    className="text-xs text-danger font-semibold"
                    onClick={() => update('options', options.slice(0, -1))}
                  >
                    - 删除最后一个
                  </button>
                )}
              </div>
            </div>
          </Field>
        </div>
      );
    }

    case 'condition':
      return (
        <div className="flex flex-col gap-3">
          <Field label="变量表达式" hint="使用 ${var.name} 引用上下文变量">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.variable ?? '')}
              onValueChange={(v) => update('variable', v)}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="比较操作符">
              <Select
                variant="bordered"
                size="sm"
                selectedKeys={[String(config.operator ?? '==')]}
                onChange={(e) => update('operator', e.target.value)}
              >
                <SelectItem key="==">等于 (==)</SelectItem>
                <SelectItem key="!=">不等于 (!=)</SelectItem>
                <SelectItem key=">">大于 (&gt;)</SelectItem>
                <SelectItem key="<">小于 (&lt;)</SelectItem>
                <SelectItem key="contains">包含 (contains)</SelectItem>
                <SelectItem key="regex">正则匹配 (regex)</SelectItem>
              </Select>
            </Field>
            <Field label="比较值">
              <Input
                variant="bordered"
                size="sm"
                value={String(config.value ?? '')}
                onValueChange={(v) => update('value', v)}
              />
            </Field>
          </div>
        </div>
      );

    case 'route':
      return (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label="选路策略">
              <Select
                variant="bordered"
                size="sm"
                selectedKeys={[String(config.strategy ?? 'lowest_cost')]}
                onChange={(e) => update('strategy', e.target.value)}
              >
                <SelectItem key="lowest_cost">最低费率</SelectItem>
                <SelectItem key="highest_quality">最高质量</SelectItem>
                <SelectItem key="round_robin">轮询</SelectItem>
                <SelectItem key="weighted">权重分配</SelectItem>
                <SelectItem key="time_based">时间路由</SelectItem>
              </Select>
            </Field>
            <Field label="回退策略">
              <Select
                variant="bordered"
                size="sm"
                selectedKeys={[String(config.fallback ?? 'reject')]}
                onChange={(e) => update('fallback', e.target.value)}
              >
                <SelectItem key="reject">拒绝</SelectItem>
                <SelectItem key="next_node">进入下一节点</SelectItem>
                <SelectItem key="play_busy">播放忙音</SelectItem>
              </Select>
            </Field>
          </div>
          {config.strategy === 'time_based' && (
            <div className="grid grid-cols-2 gap-3 p-3 bg-default-50 rounded-lg">
              <Field label="生效开始时间">
                <Input
                  variant="bordered"
                  size="sm"
                  placeholder="09:00"
                  value={String((config.time_window as { start?: string })?.start ?? '')}
                  onValueChange={(v) =>
                    update('time_window', {
                      ...(config.time_window as object),
                      start: v,
                    })
                  }
                />
              </Field>
              <Field label="生效结束时间">
                <Input
                  variant="bordered"
                  size="sm"
                  placeholder="18:00"
                  value={String((config.time_window as { end?: string })?.end ?? '')}
                  onValueChange={(v) =>
                    update('time_window', {
                      ...(config.time_window as object),
                      end: v,
                    })
                  }
                />
              </Field>
            </div>
          )}
        </div>
      );

    case 'transfer_queue':
      return (
        <div className="grid grid-cols-2 gap-3">
          <Field label="队列 ID">
            <Input
              variant="bordered"
              size="sm"
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
          <Field label="技能标签">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.skill ?? '')}
              onValueChange={(v) => update('skill', v)}
            />
          </Field>
          <Field label="排队超时 (秒)">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              value={String(config.timeout_secs ?? 60)}
              onValueChange={(v) => update('timeout_secs', Number(v) || 60)}
            />
          </Field>
        </div>
      );

    case 'transfer_ext':
      return (
        <div className="grid grid-cols-2 gap-3">
          <Field label="分机号">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.extension ?? '')}
              onValueChange={(v) => update('extension', v)}
            />
          </Field>
          <Field label="超时 (秒)">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              value={String(config.timeout_secs ?? 30)}
              onValueChange={(v) => update('timeout_secs', Number(v) || 30)}
            />
          </Field>
        </div>
      );

    case 'transfer_pstn':
      return (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label="中继 ID">
              <Input
                variant="bordered"
                size="sm"
                value={String(config.trunk_id ?? '')}
                onValueChange={(v) => update('trunk_id', v)}
              />
            </Field>
            <Field label="主叫号码">
              <Select
                variant="bordered"
                size="sm"
                selectedKeys={[String(config.caller_id ?? 'auto')]}
                onChange={(e) => update('caller_id', e.target.value)}
              >
                <SelectItem key="auto">自动分配</SelectItem>
                <SelectItem key="original">原始主叫</SelectItem>
                <SelectItem key="fixed">固定号码</SelectItem>
              </Select>
            </Field>
          </div>
          <Field label="被叫号码">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.target_number ?? '')}
              onValueChange={(v) => update('target_number', v)}
            />
          </Field>
        </div>
      );

    case 'voicemail':
      return (
        <div className="grid grid-cols-2 gap-3">
          <Field label="最大时长 (秒)">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              value={String(config.max_duration_secs ?? 60)}
              onValueChange={(v) => update('max_duration_secs', Number(v) || 60)}
            />
          </Field>
          <Field label="提示语">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.prompt ?? '')}
              onValueChange={(v) => update('prompt', v)}
            />
          </Field>
        </div>
      );

    case 'record':
      return (
        <div className="grid grid-cols-3 gap-3">
          <Field label="格式">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.format ?? 'wav')]}
              onChange={(e) => update('format', e.target.value)}
            >
              <SelectItem key="wav">WAV</SelectItem>
              <SelectItem key="mp3">MP3</SelectItem>
            </Select>
          </Field>
          <Field label="采样率">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              value={String(config.sample_rate ?? 8000)}
              onValueChange={(v) => update('sample_rate', Number(v) || 8000)}
            />
          </Field>
          <Field label="立体声">
            <Switch
              size="sm"
              isSelected={Boolean(config.stereo)}
              onChange={(e) => update('stereo', e.target.checked)}
            />
          </Field>
        </div>
      );

    case 'http_webhook':
      return (
        <div className="flex flex-col gap-3">
          <Field label="URL">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.url ?? '')}
              onValueChange={(v) => update('url', v)}
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="HTTP 方法">
              <Select
                variant="bordered"
                size="sm"
                selectedKeys={[String(config.method ?? 'POST')]}
                onChange={(e) => update('method', e.target.value)}
              >
                <SelectItem key="GET">GET</SelectItem>
                <SelectItem key="POST">POST</SelectItem>
                <SelectItem key="PUT">PUT</SelectItem>
                <SelectItem key="DELETE">DELETE</SelectItem>
              </Select>
            </Field>
            <Field label="超时 (秒)">
              <Input
                type="number"
                variant="bordered"
                size="sm"
                value={String(config.timeout_secs ?? 5)}
                onValueChange={(v) => update('timeout_secs', Number(v) || 5)}
              />
            </Field>
          </div>
        </div>
      );

    case 'set_var':
      return (
        <div className="grid grid-cols-2 gap-3">
          <Field label="变量名">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.name ?? '')}
              onValueChange={(v) => update('name', v)}
            />
          </Field>
          <Field label="变量值">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.value ?? '')}
              onValueChange={(v) => update('value', v)}
            />
          </Field>
        </div>
      );

    case 'asr':
      return (
        <div className="grid grid-cols-3 gap-3">
          <Field label="引擎">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.engine ?? 'whisper')]}
              onChange={(e) => update('engine', e.target.value)}
            >
              <SelectItem key="whisper">Whisper</SelectItem>
              <SelectItem key="paraformer">Paraformer</SelectItem>
              <SelectItem key="azure">Azure Speech</SelectItem>
            </Select>
          </Field>
          <Field label="语言">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.language ?? 'zh-CN')]}
              onChange={(e) => update('language', e.target.value)}
            >
              <SelectItem key="zh-CN">中文</SelectItem>
              <SelectItem key="en-US">英文</SelectItem>
              <SelectItem key="ja-JP">日文</SelectItem>
            </Select>
          </Field>
          <Field label="最大时长 (秒)">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              value={String(config.max_duration_secs ?? 10)}
              onValueChange={(v) => update('max_duration_secs', Number(v) || 10)}
            />
          </Field>
        </div>
      );

    case 'ai_agent':
      return (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-2 gap-3">
            <Field label="Agent ID">
              <Input
                variant="bordered"
                size="sm"
                value={String(config.agent_id ?? '')}
                onValueChange={(v) => update('agent_id', v)}
              />
            </Field>
            <Field label="最大对话轮次">
              <Input
                type="number"
                variant="bordered"
                size="sm"
                value={String(config.max_turns ?? 10)}
                onValueChange={(v) => update('max_turns', Number(v) || 10)}
              />
            </Field>
          </div>
          <Field label="系统提示词">
            <Textarea
              variant="bordered"
              size="sm"
              minRows={3}
              value={String(config.system_prompt ?? '')}
              onValueChange={(v) => update('system_prompt', v)}
            />
          </Field>
          <Field label="允许用户打断">
            <Switch
              size="sm"
              isSelected={Boolean(config.interruption)}
              onChange={(e) => update('interruption', e.target.checked)}
            />
          </Field>
        </div>
      );

    case 'loop':
      return (
        <div className="grid grid-cols-2 gap-3">
          <Field label="目标节点 ID">
            <Input
              variant="bordered"
              size="sm"
              value={String(config.target_node_id ?? '')}
              onValueChange={(v) => update('target_node_id', v)}
            />
          </Field>
          <Field label="最大循环次数">
            <Input
              type="number"
              variant="bordered"
              size="sm"
              value={String(config.max_iterations ?? 3)}
              onValueChange={(v) => update('max_iterations', Number(v) || 3)}
            />
          </Field>
        </div>
      );

    case 'hangup':
      return (
        <div className="flex flex-col gap-3">
          <Field label="挂断原因">
            <Select
              variant="bordered"
              size="sm"
              selectedKeys={[String(config.reason ?? 'normal')]}
              onChange={(e) => update('reason', e.target.value)}
            >
              <SelectItem key="normal">正常挂断</SelectItem>
              <SelectItem key="busy">忙音挂断</SelectItem>
              <SelectItem key="timeout">超时挂断</SelectItem>
              <SelectItem key="reject">拒接挂断</SelectItem>
            </Select>
          </Field>
          <Field label="播放 BYE 提示音">
            <Switch
              size="sm"
              isSelected={Boolean(config.playbye)}
              onChange={(e) => update('playbye', e.target.checked)}
            />
          </Field>
        </div>
      );

    default:
      return <div className="text-xs text-default-400">此节点类型暂无可配置项</div>;
  }
}
