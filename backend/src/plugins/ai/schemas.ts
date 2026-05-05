import { z } from 'zod'

export const aiProviderSchema = z.enum(['openai', 'anthropic'])
export const aiCredentialModeSchema = z.enum(['user_key', 'reviu_managed', 'local'])
const aiBriefPrioritySchema = z.enum(['high', 'medium', 'low'])

export const aiSettingsBodySchema = z.object({
  provider: aiProviderSchema,
  apiKey: z.string().trim().min(1),
  model: z.string().trim().min(1).optional(),
})

export const aiPrBriefBodySchema = z.object({
  owner: z.string().trim().min(1),
  repo: z.string().trim().min(1),
  pullNumber: z.number().int().positive(),
  forceRefresh: z.boolean().optional().default(false),
})

export const aiPrBriefModelOutputSchema = z.object({
  summary: z.array(z.string().trim().min(1)).min(1).max(4),
  reviewFirst: z.array(z.object({
    path: z.string().trim().min(1),
    reason: z.string().trim().min(1),
    priority: aiBriefPrioritySchema,
    target: z.object({
      type: z.literal('pr_file'),
      path: z.string().trim().min(1),
    }).optional(),
  })).max(6),
  risks: z.array(z.object({
    title: z.string().trim().min(1),
    detail: z.string().trim().min(1),
    path: z.string().trim().min(1).nullable().optional(),
    target: z.object({
      type: z.literal('pr_file'),
      path: z.string().trim().min(1),
    }).nullable().optional(),
  })).max(6),
  blockers: z.array(z.object({
    type: z.enum(['check', 'thread', 'merge', 'draft', 'outdated']),
    label: z.string().trim().min(1),
    detail: z.string().trim().min(1),
  })).max(6),
})

export const aiPrBriefSchema = aiPrBriefModelOutputSchema.extend({
  generatedAt: z.iso.datetime(),
  owner: z.string(),
  repo: z.string(),
  pullNumber: z.number().int().positive(),
  headSha: z.string(),
  contextHash: z.string(),
  provider: aiProviderSchema,
  credentialMode: aiCredentialModeSchema,
  model: z.string(),
  cached: z.boolean(),
})

export type AiProvider = z.infer<typeof aiProviderSchema>
export type AiCredentialMode = z.infer<typeof aiCredentialModeSchema>
export type AiSettingsBody = z.infer<typeof aiSettingsBodySchema>
export type AiPrBriefModelOutput = z.infer<typeof aiPrBriefModelOutputSchema>
export type AiPrBrief = z.infer<typeof aiPrBriefSchema>
