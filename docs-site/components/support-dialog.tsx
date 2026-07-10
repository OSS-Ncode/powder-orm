'use client';

import { useEffect, useState } from 'react';
import { Mail, X } from 'lucide-react';
import { SupportForm, type SupportFormCopy } from './support-form';

type DialogCopy = SupportFormCopy & {
  trigger: string;
  heading: string;
  body: string;
  close: string;
};

const T: Record<string, DialogCopy> = {
  ko: {
    trigger: '문의하기',
    heading: '문의하기',
    body: '궁금한 점이나 버그 제보, 무엇이든 아래로 남겨주세요. 입력하신 이메일로 답변드립니다.',
    emailLabel: '이메일',
    emailPlaceholder: 'you@example.com',
    messageLabel: '문의 내용',
    messagePlaceholder: '어떤 도움이 필요하신가요?',
    submit: '보내기',
    submitting: '보내는 중…',
    success: '문의가 접수됐어요. 곧 이메일로 답변드릴게요.',
    error: '전송에 실패했어요. 잠시 후 다시 시도해주세요.',
    close: '닫기',
  },
  en: {
    trigger: 'Contact us',
    heading: 'Contact us',
    body: "Questions, bug reports, anything — leave it below. We'll reply to the email you enter.",
    emailLabel: 'Email',
    emailPlaceholder: 'you@example.com',
    messageLabel: 'Message',
    messagePlaceholder: 'What do you need help with?',
    submit: 'Send',
    submitting: 'Sending…',
    success: "Your message is in. We'll reply by email soon.",
    error: 'Something went wrong. Please try again in a moment.',
    close: 'Close',
  },
  zh: {
    trigger: '联系我们',
    heading: '联系我们',
    body: '有任何问题或 bug 反馈，请在下面留言，我们会回复到您填写的邮箱。',
    emailLabel: '邮箱',
    emailPlaceholder: 'you@example.com',
    messageLabel: '留言内容',
    messagePlaceholder: '需要什么帮助？',
    submit: '发送',
    submitting: '发送中…',
    success: '已收到您的留言，我们会尽快通过邮件回复。',
    error: '发送失败，请稍后重试。',
    close: '关闭',
  },
  ja: {
    trigger: 'お問い合わせ',
    heading: 'お問い合わせ',
    body: 'ご質問やバグ報告など、なんでも下記からお送りください。入力されたメールアドレスに返信します。',
    emailLabel: 'メールアドレス',
    emailPlaceholder: 'you@example.com',
    messageLabel: 'お問い合わせ内容',
    messagePlaceholder: 'どのようなご用件でしょうか？',
    submit: '送信',
    submitting: '送信中…',
    success: 'お問い合わせを受け付けました。メールでご返信します。',
    error: '送信に失敗しました。しばらくしてから再度お試しください。',
    close: '閉じる',
  },
};

export function SupportDialog({ lang }: { lang: string }) {
  const t = T[lang] ?? T.ko;
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open]);

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="inline-flex items-center gap-1.5 transition-colors hover:text-fd-foreground"
      >
        <Mail className="size-4" />
        {t.trigger}
      </button>
      {open && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
          onClick={() => setOpen(false)}
        >
          <div
            className="relative w-full max-w-md rounded-xl border border-fd-border bg-fd-background p-6 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <button
              type="button"
              onClick={() => setOpen(false)}
              aria-label={t.close}
              className="absolute right-4 top-4 text-fd-muted-foreground transition-colors hover:text-fd-foreground"
            >
              <X className="size-4" />
            </button>
            <h2 className="text-lg font-semibold">{t.heading}</h2>
            <p className="mt-2 text-sm text-fd-muted-foreground">{t.body}</p>
            <SupportForm
              copy={{
                emailLabel: t.emailLabel,
                emailPlaceholder: t.emailPlaceholder,
                messageLabel: t.messageLabel,
                messagePlaceholder: t.messagePlaceholder,
                submit: t.submit,
                submitting: t.submitting,
                success: t.success,
                error: t.error,
              }}
            />
          </div>
        </div>
      )}
    </>
  );
}
