import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { open } from '@tauri-apps/plugin-dialog';
import {
  Check,
  Copy,
  FolderPlus,
  KeyRound,
  RefreshCw,
  ShieldCheck,
  Trash2,
} from 'lucide-react';
import {
  formatExactInt,
  formatRelativeTime,
  formatTokens,
  providerAccentColor,
  providerDisplayName,
} from '../lib/format';
import { ipc, isTauriRuntime } from '../lib/ipc';
import type {
  AppPreferences,
  CollectionMode,
  OfficialApiReconcileResult,
  ProviderConfig,
} from '../types/ipc';

const DEFAULT_PATH_HINTS: Record<string, string> = {
  claude_code: '~/.claude/projects/ 与 ~/.claude/stats-cache.json',
  codex: '~/.codex/sessions/**/rollout-*.jsonl',
  antigravity: '%APPDATA% / ~/.gemini/antigravity/brain/*/transcript.jsonl',
};

const COLLECTION_MODES: Array<{ id: CollectionMode; label: string }> = [
  { id: 'local_log', label: '本地只读日志 (Local Log)' },
  { id: 'otlp', label: '本地 OTLP 遥测 (Loopback)' },
  { id: 'official_api', label: '官方公开 Usage API' },
];

export function DataSourcesPage() {
  const queryClient = useQueryClient();

  const { data: bootstrap } = useQuery({
    queryKey: ['bootstrap'],
    queryFn: () => ipc.getBootstrapState(),
    refetchInterval: 10_000,
  });

  const { data: providers = [] } = useQuery({
    queryKey: ['provider_configs'],
    queryFn: () => ipc.getProviderConfigs(),
    refetchInterval: 10_000,
  });

  const { data: cursors = [] } = useQuery({
    queryKey: ['ingestion_cursors'],
    queryFn: () => ipc.getIngestionCursors(),
    refetchInterval: 15_000,
  });

  const [pathDrafts, setPathDrafts] = useState<Record<string, string>>({});
  const [apiKeyDrafts, setApiKeyDrafts] = useState<Record<string, string>>({});
  const [reconcileResults, setReconcileResults] = useState<
    Record<string, OfficialApiReconcileResult>
  >({});
  const [otlpPortDraft, setOtlpPortDraft] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [copiedSnippet, setCopiedSnippet] = useState(false);

  const activeOtlpPort =
    otlpPortDraft ?? String(bootstrap?.preferences.otlp_port ?? 4318);

  const otlpEnvSnippet = [
    'CLAUDE_CODE_ENABLE_TELEMETRY=1',
    `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:${activeOtlpPort}`,
    'OTEL_EXPORTER_OTLP_PROTOCOL=http/json',
    'OTEL_METRICS_EXPORTER=otlp',
    'OTEL_LOGS_EXPORTER=otlp',
  ].join('\n');

  const invalidateAllSources = async () => {
    await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
    await queryClient.invalidateQueries({ queryKey: ['provider_configs'] });
    await queryClient.invalidateQueries({ queryKey: ['ingestion_cursors'] });
    await queryClient.invalidateQueries({ queryKey: ['overview'] });
    await queryClient.invalidateQueries({ queryKey: ['tray_summary'] });
  };

  const updateProviderMutation = useMutation({
    mutationFn: (cfg: ProviderConfig) => ipc.updateProviderConfig(cfg),
    onSuccess: async () => {
      setErrorMsg(null);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '更新数据源配置失败');
    },
  });

  const scanMutation = useMutation({
    mutationFn: () => ipc.triggerScanNow(),
    onSuccess: async (report) => {
      setErrorMsg(null);
      setNotice(
        `增量扫描完成：检查 ${report.files_scanned} 个文件，新增 ${report.events_inserted} 条事件，幂等跳过 ${report.events_deduplicated} 条`
      );
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '执行增量扫描失败');
    },
  });

  const saveApiKeyMutation = useMutation({
    mutationFn: async ({ providerId, apiKey }: { providerId: string; apiKey: string }) => {
      if (!apiKey.trim()) {
        throw new Error('API Key / Admin Key 不能为空');
      }
      await ipc.setProviderApiKey(providerId, apiKey.trim());
    },
    onSuccess: async (_, vars) => {
      setApiKeyDrafts((prev) => ({ ...prev, [vars.providerId]: '' }));
      setErrorMsg(null);
      setNotice(`${providerDisplayName(vars.providerId)} 官方密钥已加密存入系统凭据管理器`);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '保存系统凭据失败');
    },
  });

  const deleteApiKeyMutation = useMutation({
    mutationFn: (providerId: string) => ipc.deleteProviderApiKey(providerId),
    onSuccess: async (_, providerId) => {
      setErrorMsg(null);
      setNotice(`已从系统凭据管理器移除 ${providerDisplayName(providerId)} 密钥`);
      await invalidateAllSources();
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '删除系统凭据失败');
    },
  });

  const reconcileMutation = useMutation({
    mutationFn: (providerId: string) => ipc.reconcileProviderOfficialApi(providerId, 7),
    onSuccess: (res) => {
      setErrorMsg(null);
      setReconcileResults((prev) => ({ ...prev, [res.provider_id]: res }));
      setNotice(
        `${providerDisplayName(res.provider_id)} 官方账单对账成功（HTTP ${res.status_code}）`
      );
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '调用官方 Usage API 对账失败');
    },
  });

  const updateOtlpPrefsMutation = useMutation({
    mutationFn: async (nextPrefs: AppPreferences) => ipc.updatePreferences(nextPrefs),
    onSuccess: async (saved) => {
      setErrorMsg(null);
      setOtlpPortDraft(String(saved.otlp_port));
      setNotice(
        saved.otlp_enabled
          ? `本地 OTLP 接收器已监听 127.0.0.1:${saved.otlp_port}`
          : '已关闭本地 OTLP 接收器'
      );
      await queryClient.invalidateQueries({ queryKey: ['bootstrap'] });
      await queryClient.invalidateQueries({ queryKey: ['preferences'] });
    },
    onError: (err: unknown) => {
      setErrorMsg(err instanceof Error ? err.message : '更新本地 OTLP 配置失败');
    },
  });

  const handleAddCustomPath = (cfg: ProviderConfig, rawPath: string) => {
    const trimmed = rawPath.trim();
    if (!trimmed) return;
    if (cfg.custom_log_paths.includes(trimmed)) {
      setPathDrafts((prev) => ({ ...prev, [cfg.provider_id]: '' }));
      return;
    }
    updateProviderMutation.mutate({
      ...cfg,
      custom_log_paths: [...cfg.custom_log_paths, trimmed],
    });
    setPathDrafts((prev) => ({ ...prev, [cfg.provider_id]: '' }));
  };

  const handleBrowseFolder = async (cfg: ProviderConfig) => {
    if (!isTauriRuntime()) {
      setNotice('目录选择器仅在桌面端原生运行时可用，可在下方输入框手动填写路径');
      return;
    }
    const selected = await open({
      directory: true,
      multiple: false,
      title: `选择 ${providerDisplayName(cfg.provider_id)} 日志目录`,
    });
    if (typeof selected === 'string' && selected.trim()) {
      handleAddCustomPath(cfg, selected);
    }
  };

  const handleRemoveCustomPath = (cfg: ProviderConfig, target: string) => {
    updateProviderMutation.mutate({
      ...cfg,
      custom_log_paths: cfg.custom_log_paths.filter((p) => p !== target),
    });
  };

  const handleCopyOtlpSnippet = async () => {
    try {
      await navigator.clipboard.writeText(otlpEnvSnippet);
      setCopiedSnippet(true);
      setTimeout(() => setCopiedSnippet(false), 1800);
    } catch {
      setNotice('请手动复制环境变量文本');
    }
  };

  const handleToggleOtlp = (enabled: boolean) => {
    const basePrefs = bootstrap?.preferences;
    if (!basePrefs) return;
    const parsedPort = Number.parseInt(activeOtlpPort, 10);
    const safePort =
      Number.isFinite(parsedPort) && parsedPort >= 1024 && parsedPort <= 65535
        ? parsedPort
        : 4318;
    updateOtlpPrefsMutation.mutate({
      ...basePrefs,
      otlp_enabled: enabled,
      otlp_port: safePort,
    });
  };

  return (
    <div className="space-y-6">
      {/* Header & Compliance Declaration Strip */}
      <section
        className="flex flex-wrap items-center justify-between gap-4 pb-5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="space-y-1.5">
          <div className="flex items-center gap-2">
            <ShieldCheck className="w-4 h-4" style={{ color: 'var(--accent-emerald)' }} />
            <h1 className="text-[16px] font-semibold tracking-tight">
              数据源配置与合规采集边界
            </h1>
          </div>
          <p className="text-[12px] max-w-3xl" style={{ color: 'var(--text-secondary)' }}>
            严格遵循三类合法采集边界：共享只读锁解析本地公开日志、仅绑定{' '}
            <code className="text-[11.5px]">127.0.0.1</code> 的官方 OTLP
            导出接收、操作系统凭据库托管的官方公开 Usage API。不读取浏览器
            Cookie、不逆向私有接口、不做流量 MITM 解密。
          </p>
        </div>

        <button
          type="button"
          onClick={() => scanMutation.mutate()}
          disabled={scanMutation.isPending}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded text-[12px] font-medium cursor-pointer disabled:opacity-50"
          style={{
            backgroundColor: 'var(--accent-primary)',
            color: '#ffffff',
          }}
        >
          <RefreshCw
            className={`w-3.5 h-3.5 ${scanMutation.isPending ? 'animate-spin' : ''}`}
          />
          <span>{scanMutation.isPending ? '扫描中…' : '立即增量扫描'}</span>
        </button>
      </section>

      {notice && (
        <div
          className="px-3 py-2 text-[12px] flex items-center justify-between"
          style={{
            borderBottom: '1px solid var(--border-subtle)',
            backgroundColor: 'var(--bg-elevated)',
            color: 'var(--accent-emerald)',
          }}
        >
          <span>{notice}</span>
          <button
            type="button"
            onClick={() => setNotice(null)}
            className="text-[11px] cursor-pointer"
            style={{ color: 'var(--text-muted)' }}
          >
            关闭
          </button>
        </div>
      )}

      {errorMsg && (
        <div
          className="px-3 py-2 text-[12px] flex items-center justify-between"
          style={{
            borderBottom: '1px solid var(--border-subtle)',
            backgroundColor: 'var(--bg-elevated)',
            color: 'var(--accent-rose)',
          }}
        >
          <span>{errorMsg}</span>
          <button
            type="button"
            onClick={() => setErrorMsg(null)}
            className="text-[11px] cursor-pointer"
            style={{ color: 'var(--text-muted)' }}
          >
            关闭
          </button>
        </div>
      )}

      {/* Section 1: Provider Data Sources */}
      <section className="space-y-2">
        <h2
          className="text-[11px] font-semibold uppercase tracking-wider"
          style={{ color: 'var(--text-muted)' }}
        >
          Provider 采集适配器
        </h2>

        <div style={{ borderTop: '1px solid var(--border-subtle)' }}>
          {providers.map((cfg) => {
            const supportsOfficialApi =
              cfg.provider_id === 'claude_code' || cfg.provider_id === 'codex';
            const reconcile = reconcileResults[cfg.provider_id];

            return (
              <div
                key={cfg.provider_id}
                className="py-5 space-y-3.5"
                style={{ borderBottom: '1px solid var(--border-subtle)' }}
              >
                <div className="flex flex-wrap items-center justify-between gap-4">
                  <div className="flex items-center gap-2.5">
                    <span
                      className="inline-block w-2 h-2 rounded-full"
                      style={{ backgroundColor: providerAccentColor(cfg.provider_id) }}
                    />
                    <span className="text-[14px] font-semibold">
                      {providerDisplayName(cfg.provider_id)}
                    </span>
                    <span
                      className="text-[11px] px-1.5 py-0.5 rounded tabular-nums"
                      style={{
                        backgroundColor: 'var(--bg-hover)',
                        color: cfg.enabled ? 'var(--accent-emerald)' : 'var(--text-muted)',
                      }}
                    >
                      {cfg.enabled ? '已启用' : '已暂停'}
                    </span>
                    <span
                      className="text-[11.5px] tabular-nums"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      最近同步：{formatRelativeTime(cfg.last_sync_at_ms)}
                    </span>
                  </div>

                  <div className="flex items-center gap-3">
                    <select
                      aria-label={`${cfg.provider_id} 采集模式`}
                      value={cfg.collection_mode}
                      onChange={(e) =>
                        updateProviderMutation.mutate({
                          ...cfg,
                          collection_mode: e.target.value as CollectionMode,
                        })
                      }
                      className="px-2 py-1 text-[11.5px] rounded outline-none cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    >
                      {COLLECTION_MODES.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.label}
                        </option>
                      ))}
                    </select>

                    <label className="flex items-center gap-1.5 text-[12px] cursor-pointer">
                      <input
                        type="checkbox"
                        checked={cfg.enabled}
                        onChange={(e) =>
                          updateProviderMutation.mutate({
                            ...cfg,
                            enabled: e.target.checked,
                          })
                        }
                      />
                      <span>启用采集</span>
                    </label>
                  </div>
                </div>

                {/* Default Path & Custom Log Directories */}
                <div className="pl-4.5 space-y-2">
                  <div className="text-[11.5px]" style={{ color: 'var(--text-secondary)' }}>
                    默认只读路径：
                    <code className="ml-1 text-[11px] select-text">
                      {DEFAULT_PATH_HINTS[cfg.provider_id] ?? '标准用户数据目录'}
                    </code>
                    {cfg.provider_id === 'antigravity' && (
                      <span className="ml-2" style={{ color: 'var(--accent-emerald)' }}>
                        · 自动跳过大体积 transcript_full.jsonl
                      </span>
                    )}
                  </div>

                  {cfg.custom_log_paths.length > 0 && (
                    <div className="space-y-1">
                      {cfg.custom_log_paths.map((customPath) => (
                        <div
                          key={customPath}
                          className="flex items-center justify-between gap-2 px-2.5 py-1 rounded text-[11.5px]"
                          style={{
                            backgroundColor: 'var(--bg-elevated)',
                            border: '1px solid var(--border-subtle)',
                          }}
                        >
                          <code className="select-text truncate">{customPath}</code>
                          <button
                            type="button"
                            onClick={() => handleRemoveCustomPath(cfg, customPath)}
                            className="inline-flex items-center gap-1 text-[11px] shrink-0 cursor-pointer hover:opacity-80"
                            style={{ color: 'var(--accent-rose)' }}
                          >
                            <Trash2 className="w-3 h-3" />
                            <span>移除</span>
                          </button>
                        </div>
                      ))}
                    </div>
                  )}

                  <div className="flex flex-wrap items-center gap-2">
                    <input
                      type="text"
                      placeholder="添加自定义日志目录或文件路径（如 C:\\Users\\...\\logs）"
                      value={pathDrafts[cfg.provider_id] ?? ''}
                      onChange={(e) =>
                        setPathDrafts((prev) => ({
                          ...prev,
                          [cfg.provider_id]: e.target.value,
                        }))
                      }
                      className="flex-1 min-w-[260px] px-2.5 py-1 text-[11.5px] rounded outline-none"
                      style={{
                        backgroundColor: 'var(--bg-input)',
                        border: '1px solid var(--border-subtle)',
                        color: 'var(--text-primary)',
                      }}
                    />
                    <button
                      type="button"
                      onClick={() =>
                        handleAddCustomPath(cfg, pathDrafts[cfg.provider_id] ?? '')
                      }
                      className="px-2.5 py-1 text-[11.5px] rounded cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-hover)',
                        color: 'var(--text-primary)',
                        border: '1px solid var(--border-subtle)',
                      }}
                    >
                      添加路径
                    </button>
                    <button
                      type="button"
                      onClick={() => handleBrowseFolder(cfg)}
                      className="inline-flex items-center gap-1 px-2.5 py-1 text-[11.5px] rounded cursor-pointer"
                      style={{
                        backgroundColor: 'var(--bg-hover)',
                        color: 'var(--text-primary)',
                        border: '1px solid var(--border-subtle)',
                      }}
                    >
                      <FolderPlus className="w-3.5 h-3.5" />
                      <span>浏览目录</span>
                    </button>
                  </div>
                </div>

                {/* Official Usage API Key & Reconciliation (Claude Code / Codex) */}
                {supportsOfficialApi && (
                  <div
                    className="pl-4.5 pt-2 flex flex-wrap items-center justify-between gap-3 text-[11.5px]"
                    style={{ borderTop: '1px dashed var(--border-subtle)' }}
                  >
                    <div className="flex items-center gap-2">
                      <KeyRound className="w-3.5 h-3.5" style={{ color: 'var(--text-muted)' }} />
                      <span style={{ color: 'var(--text-secondary)' }}>
                        {cfg.provider_id === 'claude_code'
                          ? 'Anthropic Admin API Key'
                          : 'OpenAI Admin / Usage API Key'}
                      </span>
                      <span
                        className="px-1.5 py-0.5 rounded text-[10.5px]"
                        style={{
                          backgroundColor: 'var(--bg-hover)',
                          color: cfg.has_credential
                            ? 'var(--accent-emerald)'
                            : 'var(--text-muted)',
                        }}
                      >
                        {cfg.has_credential
                          ? '已托管于 Windows 凭据管理器'
                          : '未配置密钥'}
                      </span>
                    </div>

                    <div className="flex flex-wrap items-center gap-2">
                      <input
                        type="password"
                        placeholder={
                          cfg.has_credential ? '输入新密钥以覆盖' : '粘贴官方 Admin API Key'
                        }
                        value={apiKeyDrafts[cfg.provider_id] ?? ''}
                        onChange={(e) =>
                          setApiKeyDrafts((prev) => ({
                            ...prev,
                            [cfg.provider_id]: e.target.value,
                          }))
                        }
                        className="w-56 px-2 py-1 text-[11.5px] rounded outline-none"
                        style={{
                          backgroundColor: 'var(--bg-input)',
                          border: '1px solid var(--border-subtle)',
                          color: 'var(--text-primary)',
                        }}
                      />
                      <button
                        type="button"
                        onClick={() =>
                          saveApiKeyMutation.mutate({
                            providerId: cfg.provider_id,
                            apiKey: apiKeyDrafts[cfg.provider_id] ?? '',
                          })
                        }
                        disabled={saveApiKeyMutation.isPending}
                        className="px-2.5 py-1 rounded cursor-pointer"
                        style={{
                          backgroundColor: 'var(--bg-hover)',
                          color: 'var(--text-primary)',
                          border: '1px solid var(--border-subtle)',
                        }}
                      >
                        存入凭据库
                      </button>
                      {cfg.has_credential && (
                        <>
                          <button
                            type="button"
                            onClick={() => reconcileMutation.mutate(cfg.provider_id)}
                            disabled={reconcileMutation.isPending}
                            className="px-2.5 py-1 rounded cursor-pointer"
                            style={{
                              backgroundColor: 'var(--accent-primary)',
                              color: '#ffffff',
                            }}
                          >
                            官方账单对账 (7d)
                          </button>
                          <button
                            type="button"
                            onClick={() => deleteApiKeyMutation.mutate(cfg.provider_id)}
                            className="px-2 py-1 rounded cursor-pointer"
                            style={{ color: 'var(--accent-rose)' }}
                          >
                            清除密钥
                          </button>
                        </>
                      )}
                    </div>

                    {reconcile && (
                      <div
                        className="w-full px-2.5 py-1.5 rounded text-[11px] tabular-nums flex flex-wrap items-center gap-4"
                        style={{ backgroundColor: 'var(--bg-elevated)', color: 'var(--text-secondary)' }}
                      >
                        <span>端点: {reconcile.endpoint}</span>
                        <span>时间桶: {reconcile.buckets_received}</span>
                        <span>输入: {formatTokens(reconcile.total_input_tokens)}</span>
                        <span>输出: {formatTokens(reconcile.total_output_tokens)}</span>
                        {reconcile.reported_cost_usd !== null && (
                          <span>官方账单: {reconcile.reported_cost_usd.toFixed(4)} USD</span>
                        )}
                      </div>
                    )}
                  </div>
                )}

                {cfg.last_error && (
                  <div className="pl-4.5 text-[11.5px]" style={{ color: 'var(--accent-rose)' }}>
                    最近采集告警：{cfg.last_error}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </section>

      {/* Section 2: Loopback-Only OTLP Receiver */}
      <section
        className="py-5 space-y-3"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="space-y-0.5">
            <div className="flex items-center gap-2">
              <h2 className="text-[14px] font-semibold">
                本地回环 OTLP 遥测接收器（127.0.0.1）
              </h2>
              <span
                className="text-[11px] px-1.5 py-0.5 rounded tabular-nums"
                style={{
                  backgroundColor: 'var(--bg-hover)',
                  color: bootstrap?.otlp_running
                    ? 'var(--accent-emerald)'
                    : 'var(--text-muted)',
                }}
              >
                {bootstrap?.otlp_running
                  ? `运行中 · 127.0.0.1:${bootstrap.otlp_port}`
                  : '未启动'}
              </span>
            </div>
            <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              仅监听本地回环接口（/v1/metrics、/v1/logs、/healthz），严格拒绝非 127.0.0.1 绑定
            </p>
          </div>

          <div className="flex items-center gap-2.5">
            <label className="flex items-center gap-1.5 text-[12px] tabular-nums">
              <span style={{ color: 'var(--text-secondary)' }}>端口</span>
              <input
                type="number"
                min={1024}
                max={65535}
                value={activeOtlpPort}
                onChange={(e) => setOtlpPortDraft(e.target.value)}
                className="w-20 px-2 py-1 text-[12px] rounded outline-none tabular-nums"
                style={{
                  backgroundColor: 'var(--bg-input)',
                  border: '1px solid var(--border-subtle)',
                  color: 'var(--text-primary)',
                }}
              />
            </label>
            <button
              type="button"
              onClick={() => handleToggleOtlp(!(bootstrap?.preferences.otlp_enabled ?? false))}
              disabled={updateOtlpPrefsMutation.isPending}
              className="px-3 py-1 rounded text-[12px] font-medium cursor-pointer"
              style={{
                backgroundColor: bootstrap?.preferences.otlp_enabled
                  ? 'var(--bg-hover)'
                  : 'var(--accent-primary)',
                color: bootstrap?.preferences.otlp_enabled ? 'var(--accent-rose)' : '#ffffff',
                border: '1px solid var(--border-subtle)',
              }}
            >
              {bootstrap?.preferences.otlp_enabled ? '停止 OTLP 接收' : '开启本地 OTLP'}
            </button>
          </div>
        </div>

        <div
          className="p-3 rounded flex items-start justify-between gap-4"
          style={{
            backgroundColor: 'var(--bg-elevated)',
            border: '1px solid var(--border-subtle)',
          }}
        >
          <pre className="m-0 text-[11.5px] font-mono select-text whitespace-pre-wrap" style={{ color: 'var(--text-secondary)' }}>
            {otlpEnvSnippet}
          </pre>
          <button
            type="button"
            onClick={handleCopyOtlpSnippet}
            className="inline-flex items-center gap-1 px-2 py-1 text-[11px] rounded shrink-0 cursor-pointer"
            style={{
              backgroundColor: 'var(--bg-hover)',
              color: 'var(--text-primary)',
              border: '1px solid var(--border-subtle)',
            }}
          >
            {copiedSnippet ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
            <span>{copiedSnippet ? '已复制' : '复制配置'}</span>
          </button>
        </div>
      </section>

      {/* Section 3: Ingestion Cursors High-Density Table */}
      <section className="space-y-3 pb-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-[13px] font-semibold">断点续读游标明细（ingestion_cursors）</h2>
            <p className="text-[11.5px]" style={{ color: 'var(--text-muted)' }}>
              记录每个日志文件的 file_identity、字节偏移与末行 SHA-256 指纹，保证日志轮转与重启零重复计数
            </p>
          </div>
          <span className="text-[11.5px] tabular-nums" style={{ color: 'var(--text-muted)' }}>
            共 {cursors.length} 个已跟踪文件
          </span>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-[12px]">
            <thead>
              <tr
                style={{
                  borderBottom: '1px solid var(--border-strong)',
                  color: 'var(--text-muted)',
                }}
              >
                <th className="py-2 pr-3 text-left font-medium">Provider</th>
                <th className="py-2 px-3 text-left font-medium">文件路径</th>
                <th className="py-2 px-3 text-left font-medium">文件标识</th>
                <th className="py-2 px-3 text-right font-medium">已读偏移 (Bytes)</th>
                <th className="py-2 px-3 text-left font-medium">末行指纹</th>
                <th className="py-2 pl-3 text-right font-medium">更新时间</th>
              </tr>
            </thead>
            <tbody>
              {cursors.length === 0 ? (
                <tr>
                  <td
                    colSpan={6}
                    className="py-8 text-center text-[12px]"
                    style={{ color: 'var(--text-muted)' }}
                  >
                    暂无已跟踪的日志文件，点击上方「立即增量扫描」或等待 Claude Code / Codex /
                    Antigravity 会话日志写入
                  </td>
                </tr>
              ) : (
                cursors.map((c) => (
                  <tr
                    key={c.source_path}
                    className="hover:bg-white/[0.02]"
                    style={{ borderBottom: '1px solid var(--border-subtle)' }}
                  >
                    <td className="py-2 pr-3 font-medium whitespace-nowrap">
                      {providerDisplayName(c.provider_id)}
                    </td>
                    <td
                      className="py-2 px-3 font-mono text-[11px] max-w-[360px] truncate select-text"
                      title={c.source_path}
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      {c.source_path}
                    </td>
                    <td
                      className="py-2 px-3 font-mono text-[11px] tabular-nums"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      {c.file_identity}
                    </td>
                    <td className="py-2 px-3 text-right font-mono tabular-nums">
                      {formatExactInt(c.byte_offset)}
                    </td>
                    <td
                      className="py-2 px-3 font-mono text-[11px]"
                      style={{ color: 'var(--text-muted)' }}
                    >
                      {c.last_line_hash.slice(0, 12)}
                    </td>
                    <td
                      className="py-2 pl-3 text-right tabular-nums whitespace-nowrap"
                      style={{ color: 'var(--text-secondary)' }}
                    >
                      {formatRelativeTime(c.updated_at_ms)}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
