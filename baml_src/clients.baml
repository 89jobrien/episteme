client<llm> LocalClassifier {
  provider openai-generic
  options {
    base_url env.EPISTEME_CLASSIFY_BASE_URL
    model env.EPISTEME_CLASSIFY_MODEL
    default_role "user"
    supports_streaming false
    temperature 0
  }
}

client<llm> LocalDistiller {
  provider openai-generic
  options {
    base_url env.EPISTEME_DISTILL_BASE_URL
    model env.EPISTEME_DISTILL_MODEL
    default_role "user"
    supports_streaming false
    temperature 0
  }
}
