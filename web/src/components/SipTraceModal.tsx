import { useState, useEffect, useMemo } from 'react';
import {
  Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
  Button, Spinner
} from '@heroui/react';
import { Activity } from 'lucide-react';
import { api } from '@/services/client';
import { SipFlowDiagram, type SipFlowEvent } from '@/pages/shared/call-detail';

interface SipTraceModalProps {
  isOpen: boolean;
  onClose: () => void;
  callId: string;
}

// 将 API 返回的 inbound/outbound 方向适配为 SipFlowDiagram 期望的三泳道方向。
// 请求类报文：inbound 视为 UAC→B2BUA，outbound 视为 B2BUA→UAS；
// 响应类报文：inbound 视为 UAS→B2BUA，outbound 视为 B2BUA→UAC。
// 已是三泳道格式的方向原样返回，保证向后兼容。
function adaptDirection(direction: string, message: string): string {
  if (
    direction === 'uac_to_b2bua' ||
    direction === 'b2bua_to_uac' ||
    direction === 'b2bua_to_uas' ||
    direction === 'uas_to_b2bua'
  ) {
    return direction;
  }
  const isResponse = /^\d{3}/.test(message.trim());
  if (direction === 'inbound') {
    return isResponse ? 'uas_to_b2bua' : 'uac_to_b2bua';
  }
  if (direction === 'outbound') {
    return isResponse ? 'b2bua_to_uac' : 'b2bua_to_uas';
  }
  return direction;
}

export function SipTraceModal({ isOpen, onClose, callId }: SipTraceModalProps) {
  const [loading, setLoading] = useState(false);
  const [events, setEvents] = useState<SipFlowEvent[]>([]);
  const [error, setError] = useState('');

  useEffect(() => {
    if (isOpen && callId) {
      fetchTrace();
    } else {
      setEvents([]);
      setError('');
    }
  }, [isOpen, callId]);

  const fetchTrace = async () => {
    try {
      setLoading(true);
      setError('');
      const data = await api.get<SipFlowEvent[]>(`/calls/${callId}/sip-trace`);
      setEvents(data);
    } catch (e) {
      if (e instanceof Error) setError(e.message);
    } finally {
      setLoading(false);
    }
  };

  // 适配方向字段以驱动 SipFlowDiagram 的三泳道渲染
  const adaptedEvents = useMemo(
    () => events.map((evt) => ({ ...evt, direction: adaptDirection(evt.direction, evt.message) })),
    [events],
  );

  return (
    <Modal isOpen={isOpen} onOpenChange={(o) => !o && onClose()} size="4xl" scrollBehavior="inside">
      <ModalContent className="max-w-5xl" aria-label="SIP 信令梯形图">
        <ModalHeader className="flex items-center gap-2 text-foreground border-b border-divider pb-3">
          <Activity className="w-5 h-5 text-primary" />
          <div className="flex flex-col">
            <span className="text-base font-bold">SIP 交互信令梯形图 (SIP Flow Ladder Diagram)</span>
            <span className="text-xs font-mono font-normal text-default-400">Call-ID: {callId}</span>
          </div>
        </ModalHeader>
        <ModalBody className="py-4">
          {loading ? (
            <div className="flex flex-col items-center justify-center py-16 gap-3">
              <Spinner size="lg" color="primary" />
              <p className="text-sm text-default-500">正在追踪并解析全链路 SIP 报文...</p>
            </div>
          ) : error ? (
            <div className="p-4 bg-danger/10 text-danger rounded-xl text-sm border border-danger/20">
              加载失败: {error}
            </div>
          ) : events.length === 0 ? (
            <div className="text-center py-12 text-default-400 text-sm">
              暂未捕获到该通话的 SIP 报文轨迹
            </div>
          ) : (
            <SipFlowDiagram events={adaptedEvents} />
          )}
        </ModalBody>
        <ModalFooter className="border-t border-divider pt-3">
          <Button variant="flat" onPress={onClose}>
            关闭
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
