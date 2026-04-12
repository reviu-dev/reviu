import z from 'zod'

export const updatePullRequestCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

export const issueCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

export const updateDescriptionBodySchema = z.object({
  body: z.string().transform(value => value.trim()),
})

export const createPullRequestLineCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
  path: z.string().trim().min(1, 'Missing comment path'),
  commitId: z.string().trim().min(1, 'Missing comment commit id'),
  line: z.number().int().positive(),
  side: z.enum(['LEFT', 'RIGHT']),
  startLine: z.number().int().positive().optional(),
  startSide: z.enum(['LEFT', 'RIGHT']).optional(),
})

export const createPullRequestThreadReplyBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

export const createPullRequestReviewBodySchema = z.object({
  event: z.enum(['COMMENT', 'APPROVE', 'REQUEST_CHANGES']),
  body: z.string().optional(),
})

export const createPullRequestBodySchema = z.object({
  title: z.string().trim().min(1, 'Missing pull request title'),
  base: z.string().trim().min(1, 'Missing base branch'),
  body: z.string().optional(),
  draft: z.boolean().optional(),
})

export const mergePullRequestBodySchema = z.object({
  method: z.enum(['merge', 'squash', 'rebase']),
  expectedHeadSha: z.string().trim().min(1, 'Missing expected head sha'),
  commitTitle: z.string().optional(),
  commitMessage: z.string().optional(),
})

export const pullRequestStatusMutationBodySchema = z.object({
  pullRequestId: z.string().trim().min(1, 'Missing pull request id'),
})

const pullRequestUsersMutationListSchema = z
  .array(z.string().trim().min(1, 'Missing user login'))
  .min(1, 'Missing users')
  .transform(values => [...new Set(values.map(value => value.trim()).filter(Boolean))])

export const pullRequestUsersMutationBodySchema = z.object({
  users: pullRequestUsersMutationListSchema,
})

const pullRequestLabelsMutationListSchema = z
  .array(z.string().trim().min(1, 'Missing label'))
  .min(1, 'Missing labels')
  .transform(values => [...new Set(values.map(value => value.trim()).filter(Boolean))])

export const pullRequestLabelsMutationBodySchema = z.object({
  labels: pullRequestLabelsMutationListSchema,
})

const pullRequestSearchFiltersStringListSchema = z
  .array(z.string().trim().min(1))
  .default([])
  .transform(values => [...new Set(values.map(value => value.trim()).filter(Boolean))])

export const pullRequestSearchFiltersSchema = z.object({
  repos: pullRequestSearchFiltersStringListSchema,
  labels: pullRequestSearchFiltersStringListSchema,
  authors: pullRequestSearchFiltersStringListSchema,
  assignees: pullRequestSearchFiltersStringListSchema,
  requested_reviewers: pullRequestSearchFiltersStringListSchema,
  review_status: z
    .enum(['any', 'none', 'required', 'approved', 'changes_requested'])
    .default('any'),
  include_drafts: z.boolean().default(true),
  base: z
    .string()
    .trim()
    .nullish()
    .transform(value => value || null),
  sort: z
    .enum(['updated_desc', 'created_desc', 'created_asc', 'comments_desc'])
    .default('updated_desc'),
})

export const pullRequestSearchBodySchema = z.object({
  filters: pullRequestSearchFiltersSchema,
})

export const pullRequestFilterOptionsBodySchema = z.object({
  repos: pullRequestSearchFiltersStringListSchema,
})
