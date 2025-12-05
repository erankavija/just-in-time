import type { Issue } from '../types/models';

export interface ClientSearchResult {
  issue: Issue;
  score: number;
  matches: { field: string; positions: number[] }[];
}

/**
 * Client-side issue filtering with scoring
 * Provides instant (<16ms) search results for loaded issues
 */
export function filterIssues(issues: Issue[], query: string): ClientSearchResult[] {
  if (!query.trim()) {
    return [];
  }

  const terms = query.toLowerCase().split(/\s+/).filter(t => t.length > 0);
  
  const results: ClientSearchResult[] = [];
  
  for (const issue of issues) {
    const score = calculateScore(issue, terms);
    
    if (score > 0) {
      results.push({
        issue,
        score,
        matches: findMatches(issue, terms),
      });
    }
  }
  
  // Sort by score descending
  return results.sort((a, b) => b.score - a.score);
}

/**
 * Calculate relevance score for an issue
 * Higher scores indicate better matches
 * Returns 0 if not ALL terms match
 */
export function calculateScore(issue: Issue, terms: string[]): number {
  const titleLower = issue.title.toLowerCase();
  const descLower = issue.description.toLowerCase();
  const idLower = issue.id.toLowerCase();
  const combined = `${idLower} ${titleLower} ${descLower}`;
  
  // First check: ALL terms must be present somewhere
  for (const term of terms) {
    if (!combined.includes(term)) {
      return 0; // Not all terms match
    }
  }
  
  // Now calculate score based on where matches occur
  let score = 0;
  
  for (const term of terms) {
    // ID prefix match: highest score (20 points)
    if (idLower.startsWith(term)) {
      score += 20;
    }
    
    // Title contains: high score (10 points)
    if (titleLower.includes(term)) {
      score += 10;
    }
    
    // Description contains: medium score (5 points)
    if (descLower.includes(term)) {
      score += 5;
    }
  }
  
  return score;
}

/**
 * Find match positions in issue fields
 */
function findMatches(issue: Issue, terms: string[]): { field: string; positions: number[] }[] {
  const matches: { field: string; positions: number[] }[] = [];
  const titleLower = issue.title.toLowerCase();
  const descLower = issue.description.toLowerCase();
  const idLower = issue.id.toLowerCase();
  
  for (const term of terms) {
    const titlePos = titleLower.indexOf(term);
    if (titlePos >= 0) {
      matches.push({ field: 'title', positions: [titlePos, titlePos + term.length] });
    }
    
    const descPos = descLower.indexOf(term);
    if (descPos >= 0) {
      matches.push({ field: 'description', positions: [descPos, descPos + term.length] });
    }
    
    const idPos = idLower.indexOf(term);
    if (idPos >= 0) {
      matches.push({ field: 'id', positions: [idPos, idPos + term.length] });
    }
  }
  
  return matches;
}
