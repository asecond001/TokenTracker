import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, Copy, Download, X } from 'lucide-react';
import { save } from '@tauri-apps/plugin-dialog';
import { ActionNotice } from './ActionNotice';
import { ipc, isTauriRuntime } from '../lib/ipc';
import { buildReceiptModel, drawReceipt } from '../lib/receipt';
import type { CurrencyCode, MonthlyReport } from '../types/ipc';

export interface ShareReceiptDialogProps {
  report: MonthlyReport;
  currency: CurrencyCode;
  usdToCnyRate: string;
  onClose: () => void;
}

function canvasToPngBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error('生成图片失败'));
    }, 'image/png');
  });
}

/** Receipt-style usage bill that can be copied or saved as a PNG for sharing. */
export function ShareReceiptDialog({ report, currency, usdToCnyRate, onClose }: ShareReceiptDialogProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ text: string; kind: 'success' | 'error' } | null>(null);
  const clearNotice = useCallback(() => setNotice(null), []);

  const model = useMemo(
    () => buildReceiptModel(report, currency, usdToCnyRate),
    [report, currency, usdToCnyRate]
  );

  useEffect(() => {
    if (canvasRef.current) {
      drawReceipt(canvasRef.current, model, 2);
    }
  }, [model]);

  useEffect(() => {
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [onClose]);

  const copyImage = async () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    setBusy(true);
    try {
      const blob = await canvasToPngBlob(canvas);
      if (typeof ClipboardItem === 'undefined' || !navigator.clipboard?.write) {
        throw new Error('当前环境可改用保存图片');
      }
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1800);
    } catch (err: unknown) {
      setNotice({ text: err instanceof Error ? err.message : '复制失败', kind: 'error' });
    } finally {
      setBusy(false);
    }
  };

  const saveImage = async () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    setBusy(true);
    try {
      const blob = await canvasToPngBlob(canvas);
      if (isTauriRuntime()) {
        const target = await save({
          defaultPath: model.fileName,
          filters: [{ name: 'PNG', extensions: ['png'] }],
        });
        if (!target) return;
        const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
        await ipc.saveShareImage(target, bytes);
        setNotice({ text: '账单图片已保存', kind: 'success' });
      } else {
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = model.fileName;
        a.click();
        URL.revokeObjectURL(url);
      }
    } catch (err: unknown) {
      setNotice({ text: err instanceof Error ? err.message : '保存失败', kind: 'error' });
    } finally {
      setBusy(false);
    }
  };

  return createPortal(
    <div
      className="fixed inset-0 z-[75] flex items-center justify-center p-4"
      style={{ backgroundColor: 'rgba(0, 0, 0, 0.45)' }}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-label="分享账单"
        className="liquid-popover flex flex-col max-h-full w-[420px] max-w-full"
      >
        <div
          className="flex items-center justify-between px-4 py-2.5"
          style={{ borderBottom: '1px solid var(--border-hairline)' }}
        >
          <span className="text-[13px] font-semibold">分享账单</span>
          <button
            type="button"
            aria-label="关闭"
            onClick={onClose}
            className="btn-interactive p-1 rounded-md cursor-pointer"
            style={{ color: 'var(--text-muted)' }}
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
        <div className="flex-1 min-h-0 overflow-auto flex justify-center px-4 py-4">
          <canvas
            ref={canvasRef}
            data-testid="usage-receipt-canvas"
            aria-label={model.lines
              .map((l) => ('text' in l ? l.text : 'left' in l ? `${l.left} ${l.right}` : ''))
              .filter(Boolean)
              .join('\n')}
            style={{ filter: 'drop-shadow(0 6px 18px rgba(0,0,0,0.25))' }}
          />
        </div>
        <div
          className="flex items-center justify-end gap-2 px-4 py-2.5"
          style={{ borderTop: '1px solid var(--border-hairline)' }}
        >
          <button
            type="button"
            disabled={busy}
            onClick={() => void copyImage()}
            className="btn-interactive inline-flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] rounded-lg cursor-pointer disabled:opacity-50"
            style={{ border: '1px solid var(--border-strong)', color: 'var(--text-primary)' }}
          >
            {copied ? (
              <Check className="w-3.5 h-3.5" style={{ color: 'var(--accent-emerald)' }} />
            ) : (
              <Copy className="w-3.5 h-3.5" />
            )}
            <span>{copied ? '已复制' : '复制图片'}</span>
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void saveImage()}
            className="btn-primary inline-flex items-center gap-1.5 px-3 py-1.5 text-[12px] font-medium rounded-lg cursor-pointer disabled:opacity-50"
            style={{ backgroundColor: 'var(--accent-primary)', color: '#ffffff' }}
          >
            <Download className="w-3.5 h-3.5" />
            <span>保存图片</span>
          </button>
        </div>
      </div>
      <ActionNotice text={notice?.text ?? null} kind={notice?.kind} onClose={clearNotice} />
    </div>,
    document.body
  );
}
