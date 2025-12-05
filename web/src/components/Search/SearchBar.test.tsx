import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { SearchBar } from './SearchBar';

describe('SearchBar', () => {
  it('should render search input', () => {
    render(<SearchBar onSearch={vi.fn()} />);
    const input = screen.getByPlaceholderText(/search/i);
    expect(input).toBeInTheDocument();
  });

  it('should call onSearch when typing', () => {
    const onSearch = vi.fn();
    render(<SearchBar onSearch={onSearch} />);
    
    const input = screen.getByPlaceholderText(/search/i);
    fireEvent.change(input, { target: { value: 'test' } });
    
    expect(onSearch).toHaveBeenCalledWith('test');
  });

  it('should show clear button when query is not empty', () => {
    const { rerender } = render(<SearchBar onSearch={vi.fn()} query="" />);
    expect(screen.queryByRole('button', { name: /clear/i })).not.toBeInTheDocument();
    
    rerender(<SearchBar onSearch={vi.fn()} query="test" />);
    expect(screen.getByRole('button', { name: /clear/i })).toBeInTheDocument();
  });

  it('should clear query when clear button is clicked', () => {
    const onSearch = vi.fn();
    render(<SearchBar onSearch={onSearch} query="test" />);
    
    const clearButton = screen.getByRole('button', { name: /clear/i });
    fireEvent.click(clearButton);
    
    expect(onSearch).toHaveBeenCalledWith('');
  });

  it('should show loading indicator when loading', () => {
    render(<SearchBar onSearch={vi.fn()} loading={true} />);
    expect(screen.getByText(/searching/i)).toBeInTheDocument();
  });

  it('should show error message when error exists', () => {
    render(<SearchBar onSearch={vi.fn()} error="Search failed" />);
    expect(screen.getByText(/search failed/i)).toBeInTheDocument();
  });

  it('should display result count', () => {
    render(<SearchBar onSearch={vi.fn()} query="test" resultCount={5} />);
    expect(screen.getByText(/5.*results?/i)).toBeInTheDocument();
  });
});
