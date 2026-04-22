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

export const applySuggestedChangeBodySchema = z.object({
  commitTitle: z.string().trim().min(1, 'Missing commit title'),
  commitMessage: z.string().optional(),
  expectedHeadSha: z.string().trim().min(1, 'Missing expected head sha'),
  path: z.string().trim().min(1, 'Missing suggestion path'),
  originalStartLine: z.number().int().positive(),
  originalLines: z.array(z.string()),
  suggestedLines: z.array(z.string()),
  includeCoAuthor: z.boolean().default(true),
  suggestionAuthorLogin: z.string().trim().min(1).optional(),
})

export const createPullRequestReviewBodySchema = z.object({
  event: z.enum(['COMMENT', 'APPROVE', 'REQUEST_CHANGES']),
  body: z.string().optional(),
})

const githubReactionContentSchema = z.enum([
  'CONFUSED',
  'EYES',
  'HEART',
  'HOORAY',
  'LAUGH',
  'ROCKET',
  'THUMBS_DOWN',
  'THUMBS_UP',
])

export const pullRequestReactionMutationBodySchema = z.object({
  subjectId: z.string().trim().min(1, 'Missing reaction subject id'),
  content: githubReactionContentSchema,
})

export const createPullRequestBodySchema = z.object({
  title: z.string().trim().min(1, 'Missing pull request title'),
  base: z.string().trim().min(1, 'Missing base branch'),
  body: z.string().optional(),
  draft: z.boolean().optional(),
})

export const createRepositoryBodySchema = z.object({
  name: z
    .string()
    .trim()
    .min(1, 'Missing repository name')
    .max(100, 'Repository name must be at most 100 characters')
    .regex(/^[\w.-]+$/, 'Repository name may only contain letters, numbers, dots, hyphens and underscores'),
  description: z
    .string()
    .trim()
    .max(350, 'Description must be at most 350 characters')
    .optional(),
  private: z.boolean(),
})

export const forkRepositoryBodySchema = z.object({
  organization: z.string().trim().min(1).optional(),
  name: z
    .string()
    .trim()
    .min(1)
    .max(100, 'Repository name must be at most 100 characters')
    .regex(/^[\w.-]+$/, 'Repository name may only contain letters, numbers, dots, hyphens and underscores')
    .optional(),
  defaultBranchOnly: z.boolean().default(true),
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

export const enablePullRequestAutoMergeBodySchema = z.object({
  pullRequestId: z.string().trim().min(1, 'Missing pull request id'),
  method: z.enum(['merge', 'squash', 'rebase']),
  commitTitle: z.string().optional(),
  commitMessage: z.string().optional(),
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

export const issueSearchFiltersSchema = z.object({
  repos: pullRequestSearchFiltersStringListSchema,
  labels: pullRequestSearchFiltersStringListSchema,
  authors: pullRequestSearchFiltersStringListSchema,
  assignees: pullRequestSearchFiltersStringListSchema,
  sort: z
    .enum(['updated_desc', 'created_desc', 'created_asc', 'comments_desc'])
    .default('updated_desc'),
})
