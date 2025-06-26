import React from 'react';
import ReactMarkdown from 'react-markdown';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism';
import remarkGfm from 'remark-gfm';
import { Copy, Download, FileText, Code, Database } from 'lucide-react';

interface Artifact {
  artifactId: string;
  name?: string;
  description?: string;
  parts: Array<{
    kind: string;
    text?: string;
    data?: any;
    file?: {
      uri?: string;
      bytes?: string;
      name?: string;
      mimeType?: string;
    };
  }>;
}

interface ArtifactRendererProps {
  artifact: Artifact;
  className?: string;
}

const ArtifactRenderer: React.FC<ArtifactRendererProps> = ({ artifact, className = '' }) => {
  const getArtifactType = (artifact: Artifact): string => {
    if (artifact.name?.toLowerCase().includes('markdown') || 
        artifact.description?.toLowerCase().includes('markdown')) {
      return 'markdown';
    }
    
    if (artifact.name?.toLowerCase().includes('code') ||
        artifact.name?.toLowerCase().includes('javascript') ||
        artifact.name?.toLowerCase().includes('python') ||
        artifact.name?.toLowerCase().includes('rust') ||
        artifact.name?.toLowerCase().includes('typescript')) {
      return 'code';
    }
    
    if (artifact.name?.toLowerCase().includes('json') ||
        artifact.description?.toLowerCase().includes('json')) {
      return 'json';
    }
    
    // Auto-detect based on content
    const textContent = artifact.parts
      .filter((part: any) => part.kind === 'text' && part.text)
      .map((part: any) => part.text)
      .join('\n');
    
    if (textContent.includes('# ') || textContent.includes('## ') || textContent.includes('### ')) {
      return 'markdown';
    }
    
    if (textContent.trim().startsWith('{') && textContent.trim().endsWith('}')) {
      try {
        JSON.parse(textContent);
        return 'json';
      } catch {
        // Not valid JSON
      }
    }
    
    if (textContent.includes('function ') || textContent.includes('def ') || 
        textContent.includes('class ') || textContent.includes('import ')) {
      return 'code';
    }
    
    return 'text';
  };

  const getLanguageFromName = (name?: string): string => {
    if (!name) return 'text';
    
    const lower = name.toLowerCase();
    if (lower.includes('javascript') || lower.includes('js')) return 'javascript';
    if (lower.includes('typescript') || lower.includes('ts')) return 'typescript';
    if (lower.includes('python') || lower.includes('py')) return 'python';
    if (lower.includes('rust') || lower.includes('rs')) return 'rust';
    if (lower.includes('json')) return 'json';
    if (lower.includes('html')) return 'html';
    if (lower.includes('css')) return 'css';
    if (lower.includes('yaml') || lower.includes('yml')) return 'yaml';
    if (lower.includes('bash') || lower.includes('shell')) return 'bash';
    
    return 'text';
  };

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (err) {
      console.error('Failed to copy to clipboard:', err);
    }
  };

  const downloadArtifact = (content: string, filename: string) => {
    const blob = new Blob([content], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const artifactType = getArtifactType(artifact);
  const textContent = artifact.parts
    .filter((part: any) => part.kind === 'text' && part.text)
    .map((part: any) => part.text)
    .join('\n');

  const getIcon = () => {
    switch (artifactType) {
      case 'markdown':
        return <FileText className="h-4 w-4" />;
      case 'code':
        return <Code className="h-4 w-4" />;
      case 'json':
        return <Database className="h-4 w-4" />;
      default:
        return <FileText className="h-4 w-4" />;
    }
  };

  const renderContent = () => {
    switch (artifactType) {
      case 'markdown':
        return (
          <div className="prose prose-sm max-w-none dark:prose-invert">
            <ReactMarkdown 
              remarkPlugins={[remarkGfm]}
              components={{
                code: ({node, className, children, ...props}: any) => {
                  const match = /language-(\w+)/.exec(className || '');
                  const inline = !match;
                  return !inline && match ? (
                    <SyntaxHighlighter
                      style={oneDark}
                      language={match[1]}
                      PreTag="div"
                      {...props}
                    >
                      {String(children).replace(/\n$/, '')}
                    </SyntaxHighlighter>
                  ) : (
                    <code className={className} {...props}>
                      {children}
                    </code>
                  );
                },
              }}
            >
              {textContent}
            </ReactMarkdown>
          </div>
        );
      
      case 'code':
        const language = getLanguageFromName(artifact.name);
        return (
          <SyntaxHighlighter
            language={language}
            style={oneDark}
            customStyle={{
              margin: 0,
              borderRadius: '0.375rem',
            }}
          >
            {textContent}
          </SyntaxHighlighter>
        );
      
      case 'json':
        try {
          const parsed = JSON.parse(textContent);
          const formatted = JSON.stringify(parsed, null, 2);
          return (
            <SyntaxHighlighter
              language="json"
              style={oneDark}
              customStyle={{
                margin: 0,
                borderRadius: '0.375rem',
              }}
            >
              {formatted}
            </SyntaxHighlighter>
          );
        } catch {
          return (
            <pre className="whitespace-pre-wrap font-mono text-sm bg-gray-100 p-4 rounded overflow-x-auto">
              {textContent}
            </pre>
          );
        }
      
      default:
        return (
          <pre className="whitespace-pre-wrap font-mono text-sm bg-gray-100 p-4 rounded overflow-x-auto">
            {textContent}
          </pre>
        );
    }
  };

  return (
    <div className={`border border-gray-200 rounded-lg overflow-hidden ${className}`}>
      {/* Header */}
      <div className="bg-gray-50 px-4 py-3 border-b border-gray-200 flex items-center justify-between">
        <div className="flex items-center space-x-2">
          {getIcon()}
          <div>
            <h4 className="font-medium text-gray-900">
              {artifact.name || 'Artifact'}
            </h4>
            {artifact.description && (
              <p className="text-sm text-gray-600">{artifact.description}</p>
            )}
          </div>
        </div>
        
        <div className="flex items-center space-x-2">
          <button
            onClick={() => copyToClipboard(textContent)}
            className="p-1.5 text-gray-500 hover:text-gray-700 hover:bg-gray-200 rounded"
            title="Copy to clipboard"
          >
            <Copy className="h-4 w-4" />
          </button>
          <button
            onClick={() => downloadArtifact(textContent, `${artifact.name || 'artifact'}.txt`)}
            className="p-1.5 text-gray-500 hover:text-gray-700 hover:bg-gray-200 rounded"
            title="Download"
          >
            <Download className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="p-4 bg-white">
        {renderContent()}
      </div>
    </div>
  );
};

export default ArtifactRenderer;