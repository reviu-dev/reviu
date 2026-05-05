import type { AiPrBriefModelOutput, AiProvider } from './schemas.js'
import { createAnthropic } from '@ai-sdk/anthropic'
import { createOpenAI } from '@ai-sdk/openai'
import { generateText, Output } from 'ai'
import { aiPrBriefModelOutputSchema } from './schemas.js'

interface AiProviderRequest {
  provider: AiProvider
  apiKey: string
  model: string
  systemPrompt: string
  userPrompt: string
}

interface AiProviderUsage {
  inputTokens: number | null
  outputTokens: number | null
}

interface AiProviderResult<T> {
  output: T
  usage: AiProviderUsage
}

function modelForRequest(request: AiProviderRequest) {
  if (request.provider === 'openai') {
    return createOpenAI({ apiKey: request.apiKey }).responses(request.model)
  }

  return createAnthropic({ apiKey: request.apiKey }).messages(request.model)
}

function aiSdkUsage(usage: { inputTokens?: number, outputTokens?: number }): AiProviderUsage {
  return {
    inputTokens: usage.inputTokens ?? null,
    outputTokens: usage.outputTokens ?? null,
  }
}

export async function generateAiPrBriefWithProvider(
  request: AiProviderRequest,
): Promise<AiProviderResult<AiPrBriefModelOutput>> {
  const result = await generateText({
    model: modelForRequest(request),
    output: Output.object({
      schema: aiPrBriefModelOutputSchema,
      name: 'reviu_pr_brief',
      description: 'A concise pull request review brief for Reviu.',
    }),
    system: request.systemPrompt,
    prompt: request.userPrompt,
    maxOutputTokens: 1800,
  })

  return {
    output: result.output,
    usage: aiSdkUsage(result.usage),
  }
}
