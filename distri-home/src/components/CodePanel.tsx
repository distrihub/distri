import { useEffect, useRef, useCallback } from 'react';
import CodeMirror from 'codemirror';
import 'codemirror/lib/codemirror.css';
import 'codemirror/theme/material-darker.css';
import 'codemirror/mode/javascript/javascript';
import 'codemirror/mode/python/python';
import 'codemirror/mode/shell/shell';
import 'codemirror/mode/markdown/markdown';

export type CodeLanguage = 'json' | 'javascript' | 'python' | 'shell' | 'markdown' | 'curl';

export interface CodePanelProps {
  value: string;
  onChange?: (value: string) => void;
  language: CodeLanguage;
  readOnly?: boolean;
  theme?: 'light' | 'dark';
  className?: string;
}

const languageToMode: Record<CodeLanguage, string> = {
  json: 'application/json',
  javascript: 'javascript',
  python: 'python',
  shell: 'shell',
  curl: 'shell',
  markdown: 'markdown',
};

export function CodePanel({
  value,
  onChange,
  language,
  readOnly = false,
  theme = 'dark',
  className = '',
}: CodePanelProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<CodeMirror.Editor | null>(null);
  const valueRef = useRef(value);
  const onChangeRef = useRef(onChange);
  const initializedRef = useRef(false);

  // Keep refs in sync
  useEffect(() => {
    valueRef.current = value;
  }, [value]);

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  // Initialize editor once
  useEffect(() => {
    if (!containerRef.current || initializedRef.current) return;
    initializedRef.current = true;

    const editor = CodeMirror(containerRef.current, {
      value: valueRef.current,
      mode: languageToMode[language] || 'text/plain',
      theme: theme === 'dark' ? 'material-darker' : 'default',
      lineNumbers: true,
      lineWrapping: true,
      readOnly,
      tabSize: 2,
      indentWithTabs: false,
    });

    // Make CodeMirror fill the container
    editor.setSize('100%', '100%');

    editorRef.current = editor;

    editor.on('change', (instance) => {
      const newValue = instance.getValue();
      if (newValue !== valueRef.current) {
        valueRef.current = newValue;
        onChangeRef.current?.(newValue);
      }
    });

    return () => {
      if (containerRef.current) {
        containerRef.current.innerHTML = '';
      }
      editorRef.current = null;
      initializedRef.current = false;
    };
  }, []); // Only run once on mount

  // Update mode when language changes
  useEffect(() => {
    if (editorRef.current) {
      editorRef.current.setOption('mode', languageToMode[language] || 'text/plain');
    }
  }, [language]);

  // Update theme when it changes
  useEffect(() => {
    if (editorRef.current) {
      editorRef.current.setOption('theme', theme === 'dark' ? 'material-darker' : 'default');
    }
  }, [theme]);

  // Update readOnly when it changes
  useEffect(() => {
    if (editorRef.current) {
      editorRef.current.setOption('readOnly', readOnly);
    }
  }, [readOnly]);

  // Update value when prop changes externally
  useEffect(() => {
    if (editorRef.current && editorRef.current.getValue() !== value) {
      const cursor = editorRef.current.getCursor();
      editorRef.current.setValue(value);
      editorRef.current.setCursor(cursor);
    }
  }, [value]);

  return (
    <div
      ref={containerRef}
      className={`code-panel h-full w-full ${className}`}
      style={{ fontSize: '12px' }}
    />
  );
}

export default CodePanel;
