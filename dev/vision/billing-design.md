# Payment & Billing System Design

## Overview
Stripe-powered subscription billing with invoice generation.

## Architecture

### Components
1. **Payment Gateway** - Stripe integration
2. **Subscription Management** - Plan selection and upgrades
3. **Invoice Generation** - PDF invoices via email
4. **Payment History** - Transaction logs

### Subscription Plans
- **Free**: $0/month, limited features
- **Pro**: $29/month, full features
- **Enterprise**: $99/month, custom limits

## Stripe Integration
\`\`\`typescript
// Payment Intent flow
1. Create PaymentIntent on backend
2. Confirm payment on frontend
3. Handle webhook events
4. Update subscription status
\`\`\`

### Webhook Events
- `payment_intent.succeeded`
- `payment_intent.failed`
- `subscription.created`
- `subscription.canceled`

## Database Schema
\`\`\`sql
CREATE TABLE subscriptions (
  id UUID PRIMARY KEY,
  user_id UUID REFERENCES users(id),
  plan VARCHAR(50),
  stripe_customer_id VARCHAR(255),
  stripe_subscription_id VARCHAR(255),
  status VARCHAR(50),
  current_period_end TIMESTAMP
);

CREATE TABLE invoices (
  id UUID PRIMARY KEY,
  subscription_id UUID REFERENCES subscriptions(id),
  amount_cents INTEGER,
  status VARCHAR(50),
  pdf_url TEXT,
  created_at TIMESTAMP
);
\`\`\`

## Security
- Server-side amount validation
- Webhook signature verification
- Idempotency keys for payments
- PCI compliance via Stripe
