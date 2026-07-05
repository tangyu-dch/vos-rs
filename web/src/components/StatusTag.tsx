const TEXT: Record<string, string> = {
  answered: '已接通',
  canceled: '已取消',
  failed: '失败',
};

const CLS: Record<string, string> = {
  answered: 'status-tag status-tag--answered',
  canceled: 'status-tag status-tag--canceled',
  failed: 'status-tag status-tag--failed',
};

export default function StatusTag({ status }: { status: string }) {
  return <span className={CLS[status] || 'status-tag'}>{TEXT[status] || status}</span>;
}
