import { command, type Language } from './backend.ts';
import { english } from './translations.ts';

// Resolve once before UI modules create controls; no observers run during rendering.
export const language: Language = await command('get_language').then((value): Language => value === 'en' ? 'en' : 'ja').catch((): Language => 'ja');
export function t(key: string, ...values: unknown[]) {
  const text = language === 'en' && Object.hasOwn(english, key) ? (english as Record<string, string>)[key] : key;
  return text.replace(/\{(\d+)\}/g, (match, index) => index < values.length ? String(values[index]) : match);
}

export function translateStaticPage() {
  document.documentElement.lang = language;
  const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (node.parentElement?.matches('script,style')) continue;
    const key = node.textContent?.trim() ?? '';
    if (Object.hasOwn(english, key)) node.textContent = node.textContent!.replace(key, t(key));
  }
  for (const element of document.querySelectorAll('[title],[aria-label],[placeholder],[alt],[label]')) {
    for (const attribute of ['title', 'aria-label', 'placeholder', 'alt', 'label']) {
      const value = element.getAttribute(attribute);
      if (value && Object.hasOwn(english, value)) element.setAttribute(attribute, t(value));
    }
  }
}

export function metadataValue(name: string, value = '') {
  return language === 'en' && name === 'シャッター速度' && typeof value === 'string'
    ? value.replace(/ 秒$/, ' s') : value;
}

// Preserve paths and external diagnostic details; translate only a known message prefix.
const messages = Object.keys(english).sort((a, b) => b.length - a.length);
export function errorText(error: unknown) {
  const value = String(error);
  if (language !== 'en') return value;
  return value.split('\n').map(line => {
    const prefix = line.startsWith('Error: ') ? 'Error: ' : '';
    const message = line.slice(prefix.length);
    const key = messages.find(key => message === key || message.startsWith(key + ':') || message.startsWith(key + '：'));
    return key ? prefix + t(key) + message.slice(key.length) : line;
  }).join('\n');
}
