import { useCallback, useEffect, useState } from 'react';
import { Spinner } from '@heroui/react';
import { api } from '@/services/client';
import { ErrorState } from '@/components/detail-shell';
import { SipFlowDiagram, type SipFlowEvent } from '@/components/sip-flow-diagram';

export function CallSipFlowTab({ id }: { id: string }) {
  const [events, setEvents] = useState<SipFlowEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      const value = await api.get<SipFlowEvent[]>(`/calls/${encodeURIComponent(id)}/sipflow`);
      setEvents(value);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : '加载信令流失败');
    } finally {
      setLoading(false);
    }
  }, [id]);

  useEffect(() => {
    void load();
  }, [load]);

  if (loading) {
    return (
      <div className="py-16 flex justify-center">
        <Spinner color="primary" label="正在加载信令流" />
      </div>
    );
  }
  if (error) return <ErrorState error={error} retry={load} />;
  if (events.length === 0) {
    return <div className="py-12 text-center text-small text-default-400">暂无信令报文</div>;
  }
  return <SipFlowDiagram events={events} />;
}
