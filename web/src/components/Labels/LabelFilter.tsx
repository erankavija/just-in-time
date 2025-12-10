import { useState, useMemo } from 'react';
import { LabelBadge } from './LabelBadge';

interface LabelFilterProps {
  labels: string[]; // All available labels in the graph
  selectedPatterns?: string[]; // Currently selected filter patterns
  onChange: (patterns: string[]) => void;
}

export function LabelFilter({ labels, selectedPatterns = [], onChange }: LabelFilterProps) {
  const [inputValue, setInputValue] = useState('');
  const [showSuggestions, setShowSuggestions] = useState(false);

  // Extract unique namespaces and group labels
  const labelsByNamespace = useMemo(() => {
    const grouped = new Map<string, Set<string>>();
    
    for (const label of labels) {
      const colonIndex = label.indexOf(':');
      if (colonIndex > 0) {
        const namespace = label.substring(0, colonIndex);
        const value = label.substring(colonIndex + 1);
        
        if (!grouped.has(namespace)) {
          grouped.set(namespace, new Set());
        }
        grouped.get(namespace)!.add(value);
      }
    }
    
    return grouped;
  }, [labels]);

  // Filter suggestions based on input
  const suggestions = useMemo(() => {
    if (!inputValue.trim()) {
      return labels;
    }
    
    const lower = inputValue.toLowerCase();
    return labels.filter(label => label.toLowerCase().includes(lower));
  }, [labels, inputValue]);

  const handleAddPattern = (pattern: string) => {
    if (pattern && !selectedPatterns.includes(pattern)) {
      onChange([...selectedPatterns, pattern]);
    }
    setInputValue('');
    setShowSuggestions(false);
  };

  const handleRemovePattern = (pattern: string) => {
    onChange(selectedPatterns.filter(p => p !== pattern));
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter' && inputValue.trim()) {
      handleAddPattern(inputValue.trim());
    } else if (e.key === 'Escape') {
      setShowSuggestions(false);
    }
  };

  const handleClearAll = () => {
    onChange([]);
  };

  // Generate wildcard suggestions for each namespace
  const wildcardSuggestions = useMemo(() => {
    return Array.from(labelsByNamespace.keys()).map(ns => `${ns}:*`);
  }, [labelsByNamespace]);

  return (
    <div style={{ 
      padding: '12px',
      fontFamily: 'var(--font-mono)',
      fontSize: '12px',
      borderBottom: '1px solid var(--border)',
      backgroundColor: 'var(--bg-secondary)',
    }}>
      <div style={{ 
        marginBottom: '8px',
        color: 'var(--text-secondary)',
        fontSize: '11px',
        fontWeight: 600,
        textTransform: 'uppercase',
      }}>
        🏷️ Filter by Label
      </div>

      {/* Selected patterns */}
      {selectedPatterns.length > 0 && (
        <div style={{
          display: 'flex',
          flexWrap: 'wrap',
          gap: '6px',
          marginBottom: '8px',
          alignItems: 'center',
        }}>
          {selectedPatterns.map(pattern => (
            <div
              key={pattern}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '4px',
                padding: '4px 8px',
                backgroundColor: 'var(--bg-tertiary)',
                borderRadius: '4px',
                border: '1px solid var(--border)',
              }}
            >
              <LabelBadge label={pattern} size="small" />
              <button
                onClick={() => handleRemovePattern(pattern)}
                title={`Remove filter: ${pattern}`}
                style={{
                  background: 'none',
                  border: 'none',
                  color: 'var(--text-muted)',
                  cursor: 'pointer',
                  fontSize: '12px',
                  padding: '0 2px',
                  fontFamily: 'var(--font-mono)',
                }}
              >
                ×
              </button>
            </div>
          ))}
          <button
            onClick={handleClearAll}
            style={{
              background: 'none',
              border: '1px solid var(--border)',
              color: 'var(--text-secondary)',
              cursor: 'pointer',
              fontSize: '10px',
              padding: '4px 8px',
              fontFamily: 'var(--font-mono)',
              borderRadius: '4px',
            }}
          >
            Clear All
          </button>
        </div>
      )}

      {/* Input for adding filters */}
      <div style={{ position: 'relative' }}>
        <input
          type="text"
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onFocus={() => setShowSuggestions(true)}
          onKeyDown={handleKeyDown}
          placeholder="Type to filter labels (e.g., milestone:*, epic:auth)"
          style={{
            width: '100%',
            padding: '6px 8px',
            fontFamily: 'var(--font-mono)',
            fontSize: '12px',
            backgroundColor: 'var(--bg-primary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            color: 'var(--text-primary)',
            boxSizing: 'border-box',
          }}
        />

        {/* Suggestions dropdown */}
        {showSuggestions && suggestions.length > 0 && (
          <div
            style={{
              position: 'absolute',
              top: '100%',
              left: 0,
              right: 0,
              marginTop: '4px',
              backgroundColor: 'var(--bg-tertiary)',
              border: '1px solid var(--border)',
              borderRadius: '4px',
              maxHeight: '200px',
              overflowY: 'auto',
              zIndex: 1000,
              boxShadow: '0 4px 12px rgba(0, 0, 0, 0.5)',
            }}
          >
            {/* Wildcard suggestions first */}
            {inputValue.trim() === '' && (
              <>
                <div
                  style={{
                    padding: '6px 8px',
                    fontSize: '10px',
                    color: 'var(--text-muted)',
                    textTransform: 'uppercase',
                    fontWeight: 600,
                  }}
                >
                  Wildcards
                </div>
                {wildcardSuggestions.map(pattern => (
                  <div
                    key={pattern}
                    onClick={() => handleAddPattern(pattern)}
                    style={{
                      padding: '6px 8px',
                      cursor: 'pointer',
                      backgroundColor: 'var(--bg-tertiary)',
                    }}
                    onMouseEnter={(e) => {
                      e.currentTarget.style.backgroundColor = 'var(--bg-hover)';
                    }}
                    onMouseLeave={(e) => {
                      e.currentTarget.style.backgroundColor = 'var(--bg-tertiary)';
                    }}
                  >
                    <LabelBadge label={pattern} size="small" />
                  </div>
                ))}
                <div
                  style={{
                    padding: '6px 8px',
                    fontSize: '10px',
                    color: 'var(--text-muted)',
                    textTransform: 'uppercase',
                    fontWeight: 600,
                    borderTop: '1px solid var(--border)',
                    marginTop: '4px',
                  }}
                >
                  All Labels
                </div>
              </>
            )}

            {/* Regular label suggestions */}
            {suggestions.slice(0, 20).map(label => (
              <div
                key={label}
                onClick={() => handleAddPattern(label)}
                style={{
                  padding: '6px 8px',
                  cursor: 'pointer',
                  backgroundColor: 'var(--bg-tertiary)',
                }}
                onMouseEnter={(e) => {
                  e.currentTarget.style.backgroundColor = 'var(--bg-hover)';
                }}
                onMouseLeave={(e) => {
                  e.currentTarget.style.backgroundColor = 'var(--bg-tertiary)';
                }}
              >
                <LabelBadge label={label} size="small" />
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Quick info */}
      <div
        style={{
          marginTop: '8px',
          fontSize: '10px',
          color: 'var(--text-muted)',
        }}
      >
        {selectedPatterns.length > 0 
          ? `Filtering by ${selectedPatterns.length} pattern${selectedPatterns.length > 1 ? 's' : ''}`
          : 'No filters active'}
      </div>
    </div>
  );
}
