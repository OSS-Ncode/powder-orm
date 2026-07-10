import { defineI18nUI } from 'fumadocs-ui/i18n';
import { i18n } from './i18n';

// UI-string translations + language switcher metadata for RootProvider.
export const { provider } = defineI18nUI(i18n, {
  ko: {
    displayName: '한국어',
    search: '검색',
    searchNoResult: '검색 결과가 없어요',
    toc: '이 페이지에서',
    tocNoHeadings: '제목이 없어요',
    lastUpdate: '마지막 수정',
    chooseLanguage: '언어 선택',
    nextPage: '다음',
    previousPage: '이전',
    chooseTheme: '테마',
    editOnGithub: 'GitHub에서 편집',
  },
  en: {
    displayName: 'English',
  },
  zh: {
    displayName: '简体中文',
    search: '搜索',
    searchNoResult: '没有找到结果',
    toc: '本页目录',
    tocNoHeadings: '没有标题',
    lastUpdate: '最后更新',
    chooseLanguage: '选择语言',
    nextPage: '下一页',
    previousPage: '上一页',
    chooseTheme: '主题',
    editOnGithub: '在 GitHub 上编辑',
  },
  ja: {
    displayName: '日本語',
    search: '検索',
    searchNoResult: '結果が見つかりません',
    toc: 'このページの内容',
    tocNoHeadings: '見出しがありません',
    lastUpdate: '最終更新',
    chooseLanguage: '言語を選択',
    nextPage: '次へ',
    previousPage: '前へ',
    chooseTheme: 'テーマ',
    editOnGithub: 'GitHub で編集',
  },
});
