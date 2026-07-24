import { useEffect, useState } from 'react';
import {
  Card, CardBody, Button, ButtonGroup, Chip, Avatar, Input, Modal, ModalBody, ModalHeader, ModalContent, ModalFooter,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Select, SelectItem,
  useDisclosure,
} from '@heroui/react';
import { Headset, Search, Pencil, Trash2, LayoutGrid, List, Download } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState, LoadingState, PageHeader, RefreshButton, CreateButton, EmptyState } from '@/components/detail-shell';
import { message } from '@/utils/toast';

interface AgentForm {
  agent_id: string;
  name: string;
  extension: string;
  status: string;
}

const emptyForm: AgentForm = { agent_id: '', name: '', extension: '', status: 'idle' };

export default function AgentsPage() {
  const [data, setData] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState(false);
  const [viewMode, setViewMode] = useState<'grid' | 'table'>('grid');
  const [searchKey, setSearchKey] = useState('');
  const [form, setForm] = useState<AgentForm>(emptyForm);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const { isOpen, onOpen, onClose } = useDisclosure();

  const loadData = async () => {
    setLoading(true);
    setError('');
    try {
      const res: any = await api.get('/call-center/agents');
      setData(res.items || res.data || []);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载座席列表失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, []);

  const validate = (): boolean => {
    const next: Record<string, string> = {};
    if (!form.agent_id) next.agent_id = '请填写座席工号';
    if (!form.name) next.name = '请填写座席姓名';
    if (!form.extension) next.extension = '请填写 SIP 分机号';
    setErrors(next);
    return Object.keys(next).length === 0;
  };

  const handleSave = async () => {
    if (!validate()) return;
    try {
      if (editing) {
        await api.put(`/call-center/agents/${form.agent_id}`, form);
        message.success('更新座席成功');
      } else {
        await api.post('/call-center/agents', form);
        message.success('创建座席成功');
      }
      onClose();
      loadData();
    } catch (err: any) {
      if (err?.isAxiosError || err?.message) {
        message.error(err?.message || '保存失败');
      }
    }
  };

  const handleDelete = async (agentId: string) => {
    try {
      await api.delete(`/call-center/agents/${agentId}`);
      message.success('删除成功');
      loadData();
    } catch (_err) {
      message.error('删除失败');
    }
  };

  const openCreate = () => {
    setForm(emptyForm);
    setErrors({});
    setEditing(false);
    onOpen();
  };

  const openEdit = (record: any) => {
    setForm({
      agent_id: record.agent_id,
      name: record.name,
      extension: record.extension,
      status: record.status || 'idle',
    });
    setErrors({});
    setEditing(true);
    onOpen();
  };

  // Metrics
  const totalAgents = data.length;
  const idleCount = data.filter((a) => (a.status || 'idle') === 'idle').length;
  const inCallCount = data.filter((a) => a.status === 'in_call').length;
  const busyCount = data.filter((a) => a.status === 'busy').length;
  const offlineCount = data.filter((a) => a.status === 'offline').length;

  const filteredData = data.filter(
    (a) =>
      (a.name || '').toLowerCase().includes(searchKey.toLowerCase()) ||
      (a.agent_id || '').toLowerCase().includes(searchKey.toLowerCase()) ||
      (a.extension || '').includes(searchKey)
  );

  const handleExport = async () => {
    if (!filteredData.length) {
      message.warning('当前列表无数据可导出');
      return;
    }
    try {
      setLoading(true);
      const blob = await api.blob('/call-center/agents?export=true');
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.setAttribute('href', url);
      link.setAttribute('download', `Agents_List_${new Date().toISOString().slice(0, 10)}.csv`);
      link.style.visibility = 'hidden';
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
      message.success('已从后端成功生成并下载座席列表 (CSV 格式)');
    } catch (err) {
      message.error(err instanceof Error ? err.message : '从后端导出座席数据失败');
    } finally {
      setLoading(false);
    }
  };

  const getStatusChip = (status: string) => {
    switch (status) {
      case 'idle':
        return <Chip color="success" variant="flat" size="sm">空闲 (Ready)</Chip>;
      case 'in_call':
        return <Chip color="primary" variant="flat" size="sm">通话中 (In Call)</Chip>;
      case 'busy':
        return <Chip color="warning" variant="flat" size="sm">示忙 (Busy)</Chip>;
      case 'offline':
      default:
        return <Chip color="default" variant="flat" size="sm">离线 (Offline)</Chip>;
    }
  };

  const kpis: Array<{ label: string; value: number; className: string }> = [
    { label: '总座席数', value: totalAgents, className: 'text-primary' },
    { label: '就绪/空闲', value: idleCount, className: 'text-success' },
    { label: '通话中', value: inCallCount, className: 'text-primary' },
    { label: '离线', value: offlineCount, className: 'text-default-500' },
  ];

  // 环形图：座席状态分布（自研 SVG，无新增依赖）
  const statusSegments: Array<{ key: string; label: string; value: number; colorClass: string }> = [
    { key: 'idle', label: '空闲', value: idleCount, colorClass: 'text-success' },
    { key: 'in_call', label: '通话中', value: inCallCount, colorClass: 'text-primary' },
    { key: 'busy', label: '示忙', value: busyCount, colorClass: 'text-warning' },
    { key: 'offline', label: '离线', value: offlineCount, colorClass: 'text-default-500' },
  ];
  const DONUT_R = 50;
  const DONUT_C = 2 * Math.PI * DONUT_R;
  let donutCum = 0;
  const donutSegments = statusSegments.map((seg) => {
    const length = totalAgents > 0 ? (seg.value / totalAgents) * DONUT_C : 0;
    const item = { ...seg, length, offset: -donutCum };
    donutCum += length;
    return item;
  });

  return (
    <div className="flex flex-col gap-5">
      <Card shadow="sm" className="p-2">
        <CardBody className="p-4">
          <PageHeader
            icon={Headset}
            title="座席管理"
            subtitle="实时座席状态与负载监控"
            statusChip={{ label: '实时', color: 'success', pulse: true }}
            actions={
              <>
                <RefreshButton isLoading={loading} onPress={loadData} />
                <CreateButton onPress={openCreate} label="新增座席" />
              </>
            }
          />
        </CardBody>
      </Card>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        <div className="lg:col-span-2 grid grid-cols-2 sm:grid-cols-4 gap-4">
          {kpis.map((kpi) => (
            <Card key={kpi.label} shadow="sm">
              <CardBody className="p-4">
                <div className="text-tiny font-medium text-default-500 mb-1">{kpi.label}</div>
                <div className={`text-3xl font-bold ${kpi.className}`}>{kpi.value}</div>
              </CardBody>
            </Card>
          ))}
        </div>
        <Card shadow="sm">
          <CardBody className="p-4">
            <h3 className="text-small font-semibold text-foreground mb-3">座席状态分布</h3>
            <div className="flex items-center gap-4">
              <svg
                viewBox="0 0 120 120"
                className="w-28 h-28 flex-shrink-0"
                role="img"
                aria-label="座席状态分布环形图"
              >
                <circle cx="60" cy="60" r={DONUT_R} fill="none" stroke="currentColor" strokeWidth="14" className="text-default-200" />
                {totalAgents > 0 && donutSegments.map((seg) => (
                  <circle
                    key={seg.key}
                    cx="60"
                    cy="60"
                    r={DONUT_R}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="14"
                    className={seg.colorClass}
                    strokeDasharray={`${seg.length} ${DONUT_C - seg.length}`}
                    strokeDashoffset={seg.offset}
                    transform="rotate(-90 60 60)"
                  />
                ))}
                {totalAgents === 0 ? (
                  <text x="60" y="64" textAnchor="middle" fill="currentColor" className="text-default-400" style={{ fontSize: '11px' }}>暂无数据</text>
                ) : (
                  <>
                    <text x="60" y="56" textAnchor="middle" fill="currentColor" className="text-default-500" style={{ fontSize: '10px' }}>总座席</text>
                    <text x="60" y="72" textAnchor="middle" fill="currentColor" className="text-foreground" style={{ fontSize: '20px', fontWeight: 700 }}>{totalAgents}</text>
                  </>
                )}
              </svg>
              <div className="flex flex-col gap-1.5 flex-1 min-w-0">
                {statusSegments.map((seg) => {
                  const pct = totalAgents > 0 ? ((seg.value / totalAgents) * 100).toFixed(1) : '0.0';
                  return (
                    <div key={seg.key} className="flex items-center justify-between gap-2 text-tiny">
                      <div className="flex items-center gap-1.5 min-w-0">
                        <span className={`inline-block w-2.5 h-2.5 rounded-sm ${seg.colorClass} bg-current flex-shrink-0`} />
                        <span className="text-default-600 truncate">{seg.label}</span>
                      </div>
                      <div className="flex items-center gap-1.5 flex-shrink-0">
                        <span className={`font-bold ${seg.colorClass}`}>{seg.value}</span>
                        <span className="text-default-400 font-mono">({pct}%)</span>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          </CardBody>
        </Card>
      </div>

      {error ? (
        <ErrorState error={error} retry={loadData} />
      ) : loading && data.length === 0 ? (
        <LoadingState />
      ) : (
      <Card>
        <CardBody className="gap-4 p-5">
          <div className="flex flex-wrap items-center justify-between gap-4 pb-3 border-b border-default-200">
            <Input
              placeholder="搜索工号 / 姓名 / 分机"
              className="w-64"
              size="sm"
              variant="bordered"
              startContent={<Search className="w-4 h-4 text-default-400" />}
              value={searchKey}
              onValueChange={setSearchKey}
              isClearable
            />
            <div className="flex items-center gap-2">
              <ButtonGroup>
                <Button
                  size="sm"
                  variant={viewMode === 'grid' ? 'solid' : 'light'}
                  color={viewMode === 'grid' ? 'primary' : 'default'}
                  onPress={() => setViewMode('grid')}
                  startContent={<LayoutGrid className="w-4 h-4" />}
                >
                  卡片
                </Button>
                <Button
                  size="sm"
                  variant={viewMode === 'table' ? 'solid' : 'light'}
                  color={viewMode === 'table' ? 'primary' : 'default'}
                  onPress={() => setViewMode('table')}
                  startContent={<List className="w-4 h-4" />}
                >
                  列表
                </Button>
              </ButtonGroup>

              <Button
                variant="flat"
                size="sm"
                onPress={handleExport}
                startContent={<Download className="w-4 h-4" />}
              >
                导出
              </Button>
            </div>
          </div>

          {viewMode === 'table' ? (
            <Table aria-label="座席列表">
              <TableHeader>
                <TableColumn key="name">座席人员</TableColumn>
                <TableColumn key="extension">关联 SIP 分机</TableColumn>
                <TableColumn key="status">工作状态</TableColumn>
                <TableColumn key="current_call">当前通话</TableColumn>
                <TableColumn key="actions" align="end">操作</TableColumn>
              </TableHeader>
              <TableBody items={filteredData} emptyContent={<EmptyState icon={Headset} title="暂无座席数据" description="点击「新增座席」添加第一个座席" />}>
                {(record) => (
                  <TableRow key={record.agent_id}>
                    <TableCell key="name">
                      <div className="flex items-center gap-3">
                        <Avatar name={record.name ? record.name[0] : 'U'} size="sm" />
                        <div>
                          <div className="font-semibold text-foreground">{record.name}</div>
                          <div className="text-tiny text-default-400 font-mono">ID: {record.agent_id}</div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell key="extension">
                      <Chip size="sm" variant="bordered" className="font-mono">sip:{record.extension}</Chip>
                    </TableCell>
                    <TableCell key="status">
                      {getStatusChip(record.status || 'idle')}
                    </TableCell>
                    <TableCell key="current_call">
                      <span className={record.current_call ? 'font-mono font-semibold text-primary' : 'text-default-400'}>
                        {record.current_call || '-'}
                      </span>
                    </TableCell>
                    <TableCell key="actions">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          isIconOnly
                          size="sm"
                          variant="light"
                          onPress={() => openEdit(record)}
                        >
                          <Pencil className="w-4 h-4 text-default-500" />
                        </Button>
                        <Button
                          isIconOnly
                          size="sm"
                          color="danger"
                          variant="light"
                          onPress={() => handleDelete(record.agent_id)}
                        >
                          <Trash2 className="w-4 h-4 text-danger" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          ) : (
            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4">
              {filteredData.map((agent) => (
                <Card key={agent.agent_id} isHoverable>
                  <CardBody className="p-4 gap-3">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center gap-3">
                        <Avatar name={agent.name ? agent.name[0] : 'U'} />
                        <div>
                          <div className="font-semibold text-foreground text-small">{agent.name}</div>
                          <div className="text-tiny text-default-400 font-mono">工号: {agent.agent_id}</div>
                        </div>
                      </div>
                      {getStatusChip(agent.status || 'idle')}
                    </div>

                    <div className="flex flex-col gap-1.5 text-tiny">
                      <div className="flex justify-between">
                        <span className="text-default-400">绑定分机:</span>
                        <span className="font-medium text-foreground font-mono">sip:{agent.extension}</span>
                      </div>
                      <div className="flex justify-between">
                        <span className="text-default-400">当前通话:</span>
                        <span className={agent.current_call ? 'font-semibold text-primary' : 'text-default-400'}>
                          {agent.current_call || '无通话'}
                        </span>
                      </div>
                    </div>

                    <div className="flex justify-end gap-1">
                      <Button
                        size="sm"
                        variant="flat"
                        onPress={() => openEdit(agent)}
                        startContent={<Pencil className="w-4 h-4" />}
                      >
                        编辑
                      </Button>
                      <Button
                        size="sm"
                        color="danger"
                        variant="light"
                        startContent={<Trash2 className="w-4 h-4" />}
                        onPress={() => handleDelete(agent.agent_id)}
                      >
                        删除
                      </Button>
                    </div>
                  </CardBody>
                </Card>
              ))}
            </div>
          )}
        </CardBody>
      </Card>
      )}

      <Modal isOpen={isOpen} onClose={onClose} size="lg">
        <ModalContent>
          {(onModalClose) => (
            <>
              <ModalHeader>{editing ? '编辑座席人员' : '新增座席人员'}</ModalHeader>
              <ModalBody className="gap-4 py-4">
                <Input
                  label="座席工号 (Agent ID)"
                  variant="bordered"
                  placeholder="例如 agent-101"
                  value={form.agent_id}
                  onValueChange={(v) => setForm({ ...form, agent_id: v })}
                  isDisabled={editing}
                  isRequired
                  isInvalid={!!errors.agent_id}
                  errorMessage={errors.agent_id}
                />
                <Input
                  label="座席姓名"
                  variant="bordered"
                  placeholder="例如 张三"
                  value={form.name}
                  onValueChange={(v) => setForm({ ...form, name: v })}
                  isRequired
                  isInvalid={!!errors.name}
                  errorMessage={errors.name}
                />
                <Input
                  label="关联 SIP 分机号"
                  variant="bordered"
                  placeholder="例如 8001"
                  value={form.extension}
                  onValueChange={(v) => setForm({ ...form, extension: v })}
                  isRequired
                  isInvalid={!!errors.extension}
                  errorMessage={errors.extension}
                />
                <Select
                  label="工作状态"
                  variant="bordered"
                  selectedKeys={[form.status]}
                  onChange={(e) => setForm({ ...form, status: e.target.value })}
                >
                  <SelectItem key="idle">空闲 (Ready)</SelectItem>
                  <SelectItem key="in_call">通话中 (In Call)</SelectItem>
                  <SelectItem key="busy">示忙 (Busy)</SelectItem>
                  <SelectItem key="offline">离线 (Offline)</SelectItem>
                </Select>
              </ModalBody>
              <ModalFooter>
                <Button variant="flat" onPress={onModalClose}>取消</Button>
                <Button color="primary" onPress={handleSave}>保存</Button>
              </ModalFooter>
            </>
          )}
        </ModalContent>
      </Modal>
    </div>
  );
}
