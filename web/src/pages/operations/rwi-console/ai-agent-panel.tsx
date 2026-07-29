import { Button, Tooltip } from '@heroui/react';
import { Mic, PhoneForwarded, PhoneOff, Volume2, VolumeX, Zap } from 'lucide-react';
import type { LiveCallItem } from './use-rwi-websocket';

interface CallControlPanelProps {
  currentCall: LiveCallItem;
  permissions: {
    canBarge: boolean;
    canPlay: boolean;
    canMonitor: boolean;
    canTransfer: boolean;
    canTerminate: boolean;
  };
  onBargeIn: () => void;
  onSpeak: () => void;
  onToggleListen: () => void;
  onTransfer: () => void;
  onHangup: () => void;
}

/**
 * 通话操作面板：提供强插、TTS 播报、监听、转接、挂断 5 个图标按钮。
 * 不再绑定 AI Agent 概念，统一以 SIP 媒体流操作语义呈现。
 */
export function CallControlPanel({
  currentCall,
  permissions,
  onBargeIn,
  onSpeak,
  onToggleListen,
  onTransfer,
  onHangup,
}: CallControlPanelProps) {
  const callEnded = currentCall.state === 'ended';

  return (
    <div className="overview-card p-4">
      <div className="text-xs font-semibold text-foreground mb-3 flex items-center justify-between">
        <span className="flex items-center gap-1.5">
          <Zap className="w-4 h-4 text-warning" />
          通话操作面板
        </span>
        <span className="text-[10px] text-default-400 font-normal">媒体双向注入</span>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-5 gap-2.5">
        {/* 1. 强插 BargeIn */}
        <Tooltip content="立即打断当前媒体流并接管双向通道" placement="top">
          <Button
            color="danger"
            variant="flat"
            isIconOnly
            isDisabled={!permissions.canBarge || callEnded}
            onPress={onBargeIn}
            aria-label="强插"
          >
            <Zap className="w-5 h-5" />
          </Button>
        </Tooltip>

        {/* 2. 文本播报 */}
        <Tooltip content="自定义文本由语音合成引擎合成后注入通话" placement="top">
          <Button
            color="primary"
            variant="flat"
            isIconOnly
            isDisabled={!permissions.canPlay || callEnded}
            onPress={onSpeak}
            aria-label="文本播报"
          >
            <Mic className="w-5 h-5" />
          </Button>
        </Tooltip>

        {/* 3. 监听 Listen */}
        <Tooltip content="启用/关闭本地静默监听通道" placement="top">
          <Button
            color={currentCall.listening ? 'warning' : 'primary'}
            variant={currentCall.listening ? 'solid' : 'flat'}
            isIconOnly
            isDisabled={!permissions.canMonitor || callEnded}
            onPress={onToggleListen}
            aria-label="监听"
          >
            {currentCall.listening ? (
              <VolumeX className="w-5 h-5" />
            ) : (
              <Volume2 className="w-5 h-5" />
            )}
          </Button>
        </Tooltip>

        {/* 4. 转接 Transfer */}
        <Tooltip content="盲转到指定座席分机或外部号码" placement="top">
          <Button
            color="success"
            variant="flat"
            isIconOnly
            isDisabled={!permissions.canTransfer || callEnded}
            onPress={onTransfer}
            aria-label="转接"
          >
            <PhoneForwarded className="w-5 h-5" />
          </Button>
        </Tooltip>

        {/* 5. 挂断 Hangup */}
        <Tooltip content="立即强拆并挂断当前会话" placement="top">
          <Button
            color="danger"
            variant="flat"
            isIconOnly
            isDisabled={!permissions.canTerminate || callEnded}
            onPress={onHangup}
            aria-label="挂断"
          >
            <PhoneOff className="w-5 h-5" />
          </Button>
        </Tooltip>
      </div>
    </div>
  );
}
