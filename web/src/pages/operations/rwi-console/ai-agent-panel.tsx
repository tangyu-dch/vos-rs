import { Button, Tooltip } from '@heroui/react';
import { Mic, PhoneForwarded, PhoneOff, Volume2, VolumeX, Zap } from 'lucide-react';
import type { LiveCallItem } from './use-rwi-websocket';

interface AiAgentPanelProps {
  currentCall: LiveCallItem;
  isOperatorOrAdmin: boolean;
  onBargeIn: () => void;
  onSpeak: () => void;
  onToggleListen: () => void;
  onTransfer: () => void;
  onHangup: () => void;
}

export function AiAgentPanel({
  currentCall,
  isOperatorOrAdmin,
  onBargeIn,
  onSpeak,
  onToggleListen,
  onTransfer,
  onHangup,
}: AiAgentPanelProps) {
  const callEnded = currentCall.state === 'ended';

  return (
    <div className="p-4 rounded-xl bg-content1 border border-primary/30 shadow-xl">
      <div className="text-xs font-semibold text-primary uppercase tracking-wider mb-3 flex items-center justify-between">
        <span className="flex items-center gap-1.5">
          <Zap className="w-4 h-4 text-warning" />
          AI Voice Agent 实时强控指令面板
        </span>
        <span className="text-[10px] text-primary font-normal">全双工 RTP 双向注入</span>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-5 gap-2.5">
        {/* 1. BargeIn / Interrupt */}
        <Tooltip content="立即切断 AI 当前播报，向媒体链路注入打断标记并接管" placement="top">
          <Button
            color="danger"
            variant="shadow"
            size="md"
            disabled={!isOperatorOrAdmin || callEnded}
            onPress={onBargeIn}
            startContent={<Zap className="w-4 h-4" />}
            className="font-bold bg-danger text-foreground shadow-danger/30"
          >
            BargeIn 强插
          </Button>
        </Tooltip>

        {/* 2. Speak / TTS Injection */}
        <Tooltip content="自定义输入文本并由 AI Voice Agent 立即合成语音播报" placement="top">
          <Button
            color="secondary"
            variant="flat"
            size="md"
            disabled={!isOperatorOrAdmin || callEnded}
            onPress={onSpeak}
            startContent={<Mic className="w-4 h-4" />}
            className="font-bold bg-primary/20 text-primary border border-primary/30 hover:bg-primary/30"
          >
            Speak 合成
          </Button>
        </Tooltip>

        {/* 3. Listen / Silent Tap */}
        <Tooltip content="启用/关闭本地静默监听通道，实时收听双方 RTP 音频" placement="top">
          <Button
            color={currentCall.listening ? 'warning' : 'primary'}
            variant={currentCall.listening ? 'solid' : 'flat'}
            size="md"
            disabled={callEnded}
            onPress={onToggleListen}
            startContent={currentCall.listening ? <VolumeX className="w-4 h-4" /> : <Volume2 className="w-4 h-4" />}
            className="font-bold"
          >
            {currentCall.listening ? '取消监听' : 'Listen 监听'}
          </Button>
        </Tooltip>

        {/* 4. Transfer */}
        <Tooltip content="发送 SIP REFER 盲转或协同转接至指定座席分机" placement="top">
          <Button
            color="success"
            variant="flat"
            size="md"
            disabled={!isOperatorOrAdmin || callEnded}
            onPress={onTransfer}
            startContent={<PhoneForwarded className="w-4 h-4" />}
            className="font-bold bg-success/10 text-success border border-success/30 hover:bg-success/20"
          >
            Transfer 转接
          </Button>
        </Tooltip>

        {/* 5. Hangup */}
        <Tooltip content="立即强拆并挂断当前 SIP 会话" placement="top">
          <Button
            color="danger"
            variant="flat"
            size="md"
            disabled={!isOperatorOrAdmin || callEnded}
            onPress={onHangup}
            startContent={<PhoneOff className="w-4 h-4" />}
            className="font-bold bg-danger/20 text-danger border border-danger/30 hover:bg-danger/40 col-span-2 sm:col-span-1"
          >
            Hangup 挂断
          </Button>
        </Tooltip>
      </div>
    </div>
  );
}
