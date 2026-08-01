//! search_codebase 内部常量

/// 默认最大返回结果数
pub(super) const MAX_RESULTS: usize = 20;
/// 扫描文件数硬上限（防止超大仓库长时间阻塞）
pub(super) const MAX_SCAN_FILES: usize = 20_000;
/// 跳过大于该字节数的文件（4 MiB）
pub(super) const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// 单个结果代码块最大行数（防止返回整个大文件）
pub(super) const MAX_BLOCK_LINES: usize = 80;
/// 上下文扩展时向上查找定义行的最大行数
pub(super) const MAX_BACKWARD_SEARCH: usize = 50;
/// 跳过的生成/依赖目录
pub(super) const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "__pycache__", ".venv", "venv",
    ".pytest_cache", ".mypy_cache", ".next", ".turbo", ".nuxt", ".svelte-kit",
    ".output", "coverage",
];
/// 仅搜索的代码文件扩展名（不含点）
pub(super) const CODE_EXTS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp",
    "rb", "php", "swift", "kt", "vue", "svelte", "md", "toml", "yaml", "yml", "json",
];
/// 英文停用词 + 中文停用词
pub(super) const STOP_WORDS: &[&str] = &[
    // 英文
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
    "have", "has", "had", "do", "does", "did", "will", "would", "could",
    "should", "may", "might", "must", "shall", "can",
    "to", "of", "in", "on", "at", "by", "for", "with", "about", "against",
    "between", "into", "through", "during", "before", "after", "above",
    "below", "from", "up", "down", "out", "off", "over", "under", "again",
    "further", "then", "once",
    "and", "or", "but", "if", "else", "when", "where", "why", "how",
    "all", "each", "every", "both", "few", "more", "most", "other", "some",
    "such", "no", "nor", "not", "only", "own", "same", "so", "than", "too",
    "very",
    "i", "me", "my", "we", "our", "you", "your", "he", "him", "his", "she",
    "her", "it", "its", "they", "them", "their",
    "what", "which", "who", "whom",
    // 中文
    "的", "了", "在", "是", "我", "你", "他", "她", "它", "们", "这", "那",
    "和", "与", "或", "但", "如果", "那么", "当", "为", "把", "被", "让",
    "可以", "能", "会", "要", "想", "做", "去", "来", "到", "上", "下",
    "里", "外", "中", "前", "后",
];
