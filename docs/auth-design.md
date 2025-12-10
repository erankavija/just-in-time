# Authentication System Design

## Overview
Modern JWT-based authentication with session management and OAuth support.

## Architecture

### Components
1. **Login Endpoint** - POST /api/auth/login
2. **Signup Endpoint** - POST /api/auth/signup  
3. **Token Refresh** - POST /api/auth/refresh
4. **OAuth Providers** - Google, GitHub integration

### Security
- bcrypt password hashing (cost factor: 12)
- JWT tokens with 15min expiry
- Refresh tokens with 30-day expiry
- CSRF protection with SameSite cookies

## Database Schema
\`\`\`sql
CREATE TABLE users (
  id UUID PRIMARY KEY,
  email VARCHAR(255) UNIQUE NOT NULL,
  password_hash VARCHAR(255),
  created_at TIMESTAMP,
  last_login TIMESTAMP
);

CREATE TABLE sessions (
  id UUID PRIMARY KEY,
  user_id UUID REFERENCES users(id),
  refresh_token_hash VARCHAR(255),
  expires_at TIMESTAMP,
  created_at TIMESTAMP
);
\`\`\`

## API Design
- RESTful endpoints
- JSON payloads
- HTTP-only cookies for tokens
- Rate limiting: 5 attempts per 15 minutes

## Testing Strategy
- Unit tests for password validation
- Integration tests for login flow
- E2E tests for OAuth flows
