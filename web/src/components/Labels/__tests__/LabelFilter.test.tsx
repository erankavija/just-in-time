import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { LabelFilter } from '../LabelFilter';

describe('LabelFilter', () => {
  it('should render without crashing', () => {
    render(<LabelFilter labels={[]} onChange={vi.fn()} />);
    expect(screen.getByText(/filter by label/i)).toBeDefined();
  });

  it('should display available labels grouped by namespace', () => {
    const labels = [
      'milestone:v1.0',
      'milestone:v2.0',
      'epic:auth',
      'epic:billing',
      'component:api',
    ];
    render(<LabelFilter labels={labels} onChange={vi.fn()} />);
    
    // Component should organize labels by namespace
    expect(screen.getByText(/filter by label/i)).toBeDefined();
  });

  it('should allow selecting a single label', () => {
    const onChange = vi.fn();
    const labels = ['milestone:v1.0', 'epic:auth'];
    
    render(<LabelFilter labels={labels} onChange={onChange} selectedPatterns={[]} />);
    
    const input = screen.getByPlaceholderText(/type to filter/i);
    fireEvent.change(input, { target: { value: 'milestone:v1.0' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    
    expect(onChange).toHaveBeenCalledWith(['milestone:v1.0']);
  });

  it('should allow selecting multiple labels', () => {
    const onChange = vi.fn();
    const labels = ['milestone:v1.0', 'epic:auth', 'component:api'];
    
    render(<LabelFilter labels={labels} onChange={onChange} selectedPatterns={['milestone:v1.0']} />);
    
    const input = screen.getByPlaceholderText(/type to filter/i);
    fireEvent.change(input, { target: { value: 'epic:auth' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    
    expect(onChange).toHaveBeenCalledWith(['milestone:v1.0', 'epic:auth']);
  });

  it('should allow removing selected labels', () => {
    const onChange = vi.fn();
    const labels = ['milestone:v1.0', 'epic:auth'];
    
    const { container } = render(
      <LabelFilter 
        labels={labels} 
        onChange={onChange} 
        selectedPatterns={['milestone:v1.0', 'epic:auth']} 
      />
    );
    
    // Find and click remove button for first selected label
    const removeButtons = container.querySelectorAll('button[title*="Remove"]');
    expect(removeButtons.length).toBeGreaterThan(0);
    
    fireEvent.click(removeButtons[0]);
    
    // Should be called with one less label
    expect(onChange).toHaveBeenCalled();
    const lastCall = onChange.mock.calls[onChange.mock.calls.length - 1];
    expect(lastCall[0].length).toBe(1);
  });

  it('should support wildcard patterns', () => {
    const onChange = vi.fn();
    const labels = ['milestone:v1.0', 'milestone:v2.0'];
    
    render(<LabelFilter labels={labels} onChange={onChange} selectedPatterns={[]} />);
    
    const input = screen.getByPlaceholderText(/type to filter/i);
    fireEvent.change(input, { target: { value: 'milestone:*' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    
    expect(onChange).toHaveBeenCalledWith(['milestone:*']);
  });

  it('should clear all filters', () => {
    const onChange = vi.fn();
    const labels = ['milestone:v1.0', 'epic:auth'];
    
    render(
      <LabelFilter 
        labels={labels} 
        onChange={onChange} 
        selectedPatterns={['milestone:v1.0', 'epic:auth']} 
      />
    );
    
    const clearButton = screen.getByText(/clear all/i);
    fireEvent.click(clearButton);
    
    expect(onChange).toHaveBeenCalledWith([]);
  });

  it('should not show clear button when no filters active', () => {
    const labels = ['milestone:v1.0'];
    
    render(<LabelFilter labels={labels} onChange={vi.fn()} selectedPatterns={[]} />);
    
    expect(screen.queryByText(/clear all/i)).toBeNull();
  });

  it('should show count of active filters', () => {
    const labels = ['milestone:v1.0', 'epic:auth'];
    
    const { container } = render(
      <LabelFilter 
        labels={labels} 
        onChange={vi.fn()} 
        selectedPatterns={['milestone:v1.0', 'epic:auth']} 
      />
    );
    
    // Should display filter count
    expect(screen.getByText(/filtering by 2 patterns/i)).toBeDefined();
    
    // Should render LabelBadge components for both patterns
    const badges = container.querySelectorAll('[title*="filter"]');
    expect(badges.length).toBe(2);
  });
});
