import { useEffect, useState } from 'react';
import {
  Card, CardBody, Button, Chip, Input, Modal, ModalBody, ModalHeader, ModalContent, ModalFooter,
  Table, TableHeader, TableColumn, TableBody, TableRow, TableCell, Select, SelectItem,
  useDisclosure,
} from '@heroui/react';
import { Plus, RefreshCw, Search, Pencil, Trash2, Music } from 'lucide-react';
import { api } from '@/services/client';
import { ErrorState, LoadingState } from '@/components/detail-shell';
import { message } from '@/utils/toast';

interface QueueForm {
  id: string;
  name: string;
  strategy: string;
  moh_file: string;
  max_wait_secs: number;
  agents: string[];
}

const emptyForm: QueueForm = {
  id: '',
  name: '',
  strategy: 'longest_idle',
  moh_file: 'moh.wav',
  max_wait_secs: 300,
  agents: [],
};

const strategyOptions = [
  { value: 'longest_idle', label: '最长空闲优先 (Longest Idle)' },
  { value: 'round_robin', label: '轮询分发 (Round Robin)' },
  { value: 'ring_all', label: '群响 (Ring All)' },
  { value: 'random', label: '随机分配 (Random)' },
];

export default function QueuesPage() {
  const [data, setData] = useState<any[]>([]);
  const [agentsList, setAgentsList] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [searchKey, setSearchKey] = useState('');
  const [form, setForm] = useState<QueueForm>(emptyForm);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState(false);
  const { isOpen, onOpen, onClose } = useDisclosure();

  const loadData = async () => {
    setLoading(true);
    setError('');
    try {
      const [qRes, aRes]: [any, any] = await Promise.all([
        api.get('/call-center/queues'),
        api.get('/call-center/agents'),
      ]);
      setData(qRes.items || qRes.data || []);
      setAgentsList(aRes.items || aRes.data || []);
    } catch (err) {
      setError(err instanceof Error ? err.message : '加载呼叫队列列表失败');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, []);

  const validate = (): boolean => {
    const next: Record<string, string> = {};
    if (!form.id) next.id = '请填写队列 ID';
    if (!form.name) next.name = '请填写队列名称';
    setErrors(next);
    return Object.keys(next).length === 0;
  };

  const handleSave = async () => {
    if (!validate()) return;
    const payload = {
      id: form.id,
      name: form.name,
      strategy: form.strategy || 'longest_idle',
      moh_file: form.moh_file || 'moh.wav',
      max_wait_secs: form.max_wait_secs || 300,
      agents: form.agents || [],
    };
    try {
      if (editing) {
        await api.put(`/call-center/queues/${form.id}`, payload);
        message.success('更新呼叫队列成功');
      } else {
        await api.post('/call-center/queues', payload);
        message.success('创建呼叫队列成功');
      }
      onClose();
      loadData();
    } catch (err: any) {
      if (err?.isAxiosError || err?.message) {
        message.error(err?.message || '保存失败');
      }
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await api.delete(`/call-center/queues/${id}`);
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
      id: record.id,
      name: record.name,
      strategy: record.strategy || 'longest_idle',
      moh_file: record.moh_file || 'moh.wav',
      max_wait_secs: record.max_wait_secs || 300,
      agents: record.agents || [],
    });
    setErrors({});
    setEditing(true);
    onOpen();
  };

  const filteredData = data.filter(
    (q) =>
      (q.name || '').toLowerCase().includes(searchKey.toLowerCase()) ||
      (q.id || '').toLowerCase().includes(searchKey.toLowerCase())
  );

  const getStrategyChip = (strategy: string) => {
    switch (strategy) {
      case 'round_robin':
        return <Chip color="primary" variant="flat" size="sm">轮询分发 (Round Robin)</Chip>;
      case 'ring_all':
        return <Chip color="secondary" variant="flat" size="sm">群响 (Ring All)</Chip>;
      case 'random':
        return <Chip color="warning" variant="flat" size="sm">随机 (Random)</Chip>;
      case 'longest_idle':
      default:
        return <Chip color="success" variant="flat" size="sm">最长空闲优先 (Longest Idle)</Chip>;
    }
  };

  const kpis: Array<{ label: string; value: number | string; className: string }> = [
    { label: '呼叫队列总数', value: data.length, className: 'text-primary' },
    { label: '在线绑定座席人次', value: data.reduce((acc, q) => acc + (Array.isArray(q.agents) ? q.agents.length : 0), 0), className: 'text-success' },
    { label: '可用分发算法', value: '4 种策略', className: 'text-secondary' },
  ];

  return (
    <div className="flex flex-col gap-5">
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {kpis.map((kpi) => (
          <Card key={kpi.label}>
            <CardBody className="p-4">
              <div className="text-tiny font-medium text-default-500 mb-1">{kpi.label}</div>
              <div className={`text-3xl font-bold ${kpi.className}`}>{kpi.value}</div>
            </CardBody>
          </Card>
        ))}
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
              placeholder="搜索队列 ID / 名称"
              className="w-64"
              size="sm"
              variant="bordered"
              startContent={<Search className="w-4 h-4 text-default-400" />}
              value={searchKey}
              onValueChange={setSearchKey}
              isClearable
            />
            <div className="flex items-center gap-2">
              <Button
                variant="flat"
                size="sm"
                isLoading={loading}
                onPress={loadData}
                startContent={<RefreshCw className="w-4 h-4" />}
              >
                刷新
              </Button>
              <Button
                color="primary"
                size="sm"
                onPress={openCreate}
                startContent={<Plus className="w-4 h-4" />}
              >
                新建队列
              </Button>
            </div>
          </div>

          <Table aria-label="呼叫队列列表">
            <TableHeader>
              <TableColumn key="id">队列 ID</TableColumn>
              <TableColumn key="name">队列名称</TableColumn>
              <TableColumn key="strategy">分配策略</TableColumn>
              <TableColumn key="moh_file">MOH 音乐文件</TableColumn>
              <TableColumn key="max_wait_secs">最大等待 (秒)</TableColumn>
              <TableColumn key="agents">绑定座席数</TableColumn>
              <TableColumn key="actions" align="end">操作</TableColumn>
            </TableHeader>
            <TableBody items={filteredData} emptyContent="暂无呼叫队列数据">
              {(item) => (
                <TableRow key={item.id}>
                  <TableCell key="id">
                    <span className="font-mono font-semibold text-foreground">{item.id}</span>
                  </TableCell>
                  <TableCell key="name">{item.name}</TableCell>
                  <TableCell key="strategy">{getStrategyChip(item.strategy)}</TableCell>
                  <TableCell key="moh_file">
                    <div className="flex items-center gap-1.5 text-default-600 font-mono">
                      <Music className="w-3.5 h-3.5 text-default-400" />
                      <span>{item.moh_file || 'moh.wav'}</span>
                    </div>
                  </TableCell>
                  <TableCell key="max_wait_secs">{item.max_wait_secs || 300}s</TableCell>
                  <TableCell key="agents">
                    <Chip size="sm" variant="bordered">
                      {Array.isArray(item.agents) ? `${item.agents.length} 个座席` : '0 个座席'}
                    </Chip>
                  </TableCell>
                  <TableCell key="actions">
                    <div className="flex items-center justify-end gap-1">
                      <Button isIconOnly size="sm" variant="light" onPress={() => openEdit(item)}>
                        <Pencil className="w-4 h-4 text-default-500" />
                      </Button>
                      <Button
                        isIconOnly
                        size="sm"
                        color="danger"
                        variant="light"
                        onPress={() => handleDelete(item.id)}
                      >
                        <Trash2 className="w-4 h-4 text-danger" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </CardBody>
      </Card>
      )}

      <Modal isOpen={isOpen} onClose={onClose} size="lg">
        <ModalContent>
          {(onModalClose) => (
            <>
              <ModalHeader>{editing ? '编辑呼叫队列' : '新建呼叫队列'}</ModalHeader>
              <ModalBody className="gap-4 py-4">
                <Input
                  label="队列 ID"
                  variant="bordered"
                  placeholder="例如 support-queue"
                  value={form.id}
                  onValueChange={(v) => setForm({ ...form, id: v })}
                  isDisabled={editing}
                  isRequired
                  isInvalid={!!errors.id}
                  errorMessage={errors.id}
                />
                <Input
                  label="队列名称"
                  variant="bordered"
                  placeholder="例如 技术支持队列"
                  value={form.name}
                  onValueChange={(v) => setForm({ ...form, name: v })}
                  isRequired
                  isInvalid={!!errors.name}
                  errorMessage={errors.name}
                />
                <Select
                  label="座席分配策略"
                  variant="bordered"
                  selectedKeys={[form.strategy]}
                  onChange={(e) => setForm({ ...form, strategy: e.target.value })}
                >
                  {strategyOptions.map((opt) => (
                    <SelectItem key={opt.value}>{opt.label}</SelectItem>
                  ))}
                </Select>
                <Input
                  label="MOH 背景音乐文件"
                  variant="bordered"
                  placeholder="例如 /var/lib/vos/sounds/moh.wav"
                  value={form.moh_file}
                  onValueChange={(v) => setForm({ ...form, moh_file: v })}
                />
                <Input
                  type="number"
                  label="最大排队等待超时 (秒)"
                  variant="bordered"
                  placeholder="10 - 3600"
                  value={String(form.max_wait_secs)}
                  onValueChange={(v) => setForm({ ...form, max_wait_secs: Number(v) || 300 })}
                  min={10}
                  max={3600}
                />
                <Select
                  label="绑定座席人员"
                  variant="bordered"
                  selectionMode="multiple"
                  selectedKeys={new Set(form.agents)}
                  onSelectionChange={(keys) => {
                    const next = Array.from(keys as Iterable<string>);
                    setForm({ ...form, agents: next });
                  }}
                  placeholder="选择加入该队列的座席"
                >
                  {agentsList.map((a: any) => (
                    <SelectItem key={a.agent_id}>
                      {a.name} ({a.agent_id} - sip:{a.extension})
                    </SelectItem>
                  ))}
                </Select>
              </ModalBody>
              <ModalFooter>
                <Button variant="light" onPress={onModalClose}>取消</Button>
                <Button color="primary" onPress={handleSave}>保存</Button>
              </ModalFooter>
            </>
          )}
        </ModalContent>
      </Modal>
    </div>
  );
}
