// 运营监控 - 活跃通话
// 从 console.tsx 拆分

import { useCallback, useEffect, useState } from 'react';
import {
  Button, Card, CardBody, Chip, Table, TableHeader, TableColumn, TableBody, TableRow, TableCell,
} from '@heroui/react';
import { RefreshCw, Eye, PhoneOff } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { api } from '@/services/client';
import { useAuth } from '@/auth/AuthContext';
import { canWriteDomain } from '@/services/auth';
import type { Entity } from '@/services/resources';
import { ErrorState } from '@/components/detail-shell';
import { message } from '@/utils/toast';
import {
  ConfirmDialog, usePageVisibility,
} from '@/pages/shared/resource-workspace';
import { callDetailText, entityId, valueText } from '@/pages/shared/format';

export function ActiveCallsPage() {
  const [rows, setRows] = useState<Entity[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [confirmRow, setConfirmRow] = useState<Entity | null>(null);
  const [saving, setSaving] = useState(false);
  const navigate = useNavigate();
  const { session } = useAuth();
  const isVisible = usePageVisibility();

  const load = useCallback(async () => {
    setLoading(true);
    setError('');
    try {
      setRows(await api.get<Entity[]>('/calls/active'));
    } catch (e) {
      setError(e instanceof Error ? e.message : '加载失败');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!isVisible) return;
    void load();
    const timer = window.setInterval(load, 10000);
    return () => window.clearInterval(timer);
  }, [load, isVisible]);

  const terminate = async (row: Entity) => {
    try {
      setSaving(true);
      await api.post(`/calls/${encodeURIComponent(entityId(row, 'call_id'))}/actions/terminate`);
      message.success('已发送强制挂断指令');
      await load();
    } catch (e) {
      message.error(e instanceof Error ? e.message : '操作失败');
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card shadow="sm" className="p-2">
      <CardBody className="p-4 flex flex-col gap-4">
        <div className="flex flex-wrap items-center justify-between gap-4 pb-4 border-b border-divider">
          <div>
            <div className="flex items-center gap-2 mb-1">
              <h2 className="text-base font-bold text-foreground">活跃通话监控</h2>
              <Chip color="success" size="sm" variant="flat">10s 实时刷新</Chip>
            </div>
            <p className="text-tiny text-default-500">实时查看正在建立与通话中的会话，支持强拆挂断与 SIP 事务分析</p>
          </div>
          <Button variant="flat" size="sm" isLoading={loading} onPress={load} startContent={<RefreshCw className="w-4 h-4" />}>
            刷新
          </Button>
        </div>

        {error ? (
          <ErrorState error={error} retry={load} />
        ) : (
          <Table aria-label="活跃通话列表" isStriped>
            <TableHeader>
              <TableColumn key="call_id">通话 ID</TableColumn>
              <TableColumn key="caller">主叫号码</TableColumn>
              <TableColumn key="callee">被叫号码</TableColumn>
              <TableColumn key="state">状态</TableColumn>
              <TableColumn key="started_at_ms">开始时间</TableColumn>
              <TableColumn key="gateway">中继网关</TableColumn>
              <TableColumn key="actions" align="end">操作</TableColumn>
            </TableHeader>
            <TableBody items={rows} emptyContent="当前无活跃通话">
              {(row) => (
                <TableRow key={entityId(row, 'call_id')}>
                  <TableCell>
                    <span className="font-mono text-foreground font-bold">{entityId(row, 'call_id')}</span>
                  </TableCell>
                  <TableCell>{valueText(row.caller)}</TableCell>
                  <TableCell>{valueText(row.callee)}</TableCell>
                  <TableCell>
                    <Chip
                      size="sm"
                      color={['active', 'answered', 'in_call'].includes(String(row.state).toLowerCase()) ? 'success' : 'warning'}
                      variant="flat"
                    >
                      {valueText(row.state)}
                    </Chip>
                  </TableCell>
                  <TableCell>{callDetailText(row.started_at_ms, 'started_at_ms')}</TableCell>
                  <TableCell>{valueText(row.gateway)}</TableCell>
                  <TableCell>
                    <div className="flex items-center justify-end gap-1">
                      <Button isIconOnly size="sm" variant="light" onPress={() => navigate(`/calls/${entityId(row, 'call_id')}`)}>
                        <Eye className="w-4 h-4 text-default-500" />
                      </Button>
                      {session && canWriteDomain(session.role, 'operations') && (
                        <Button
                          size="sm"
                          color="danger"
                          variant="flat"
                          startContent={<PhoneOff className="w-3.5 h-3.5" />}
                          onPress={() => setConfirmRow(row)}
                        >
                          强拆挂断
                        </Button>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        )}
      </CardBody>

      <ConfirmDialog
        open={Boolean(confirmRow)}
        title="确认强制挂断"
        message="确认强制挂断此通话？该操作会立即终止会话。"
        loading={saving}
        onConfirm={async () => {
          if (confirmRow) await terminate(confirmRow);
          setConfirmRow(null);
        }}
        onClose={() => setConfirmRow(null)}
      />
    </Card>
  );
}
