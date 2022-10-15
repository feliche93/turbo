use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use turbo_tasks_fs::FileSystemPathVc;
use turbopack_core::chunk::ChunkingContextVc;
use turbopack_dev_server::source::{
    asset_graph::AssetGraphContentSourceVc,
    conditional::ConditionalContentSourceVc,
    lazy_instatiated::{GetContentSource, GetContentSourceVc, LazyInstantiatedContentSource},
    ContentSource, ContentSourceData, ContentSourceDataFilter, ContentSourceDataVary,
    ContentSourceDataVaryVc, ContentSourceResult, ContentSourceResultVc, ContentSourceVc,
};
use turbopack_ecmascript::{chunk::EcmascriptChunkPlaceablesVc, EcmascriptModuleAssetVc};

use super::{external_asset_entrypoints, render_static, RenderData};
use crate::path_regex::PathRegexVc;

/// Trait that allows to get the entry module for rendering something in Node.js
#[turbo_tasks::value_trait]
pub trait NodeRenderer {
    fn module(&self) -> EcmascriptModuleAssetVc;
}

/// Creates a content source that renders something in Node.js with the passed
/// `renderer` when it matches a `path_regex`. Once rendered it serves
/// all assets referenced by the `renderer` that are within the `server_root`.
/// It needs a temporary directory (`intermediate_output_path`) to place file
/// for Node.js execution during rendering. The `chunking_context` should emit
/// to this directory.
#[turbo_tasks::function]
pub fn create_node_rendered_source(
    server_root: FileSystemPathVc,
    path_regex: PathRegexVc,
    renderer: NodeRendererVc,
    chunking_context: ChunkingContextVc,
    runtime_entries: EcmascriptChunkPlaceablesVc,
    intermediate_output_path: FileSystemPathVc,
) -> ContentSourceVc {
    let source = NodeRenderContentSource {
        server_root,
        path_regex,
        renderer,
        chunking_context,
        runtime_entries,
        intermediate_output_path,
    }
    .cell();
    ConditionalContentSourceVc::new(
        source.into(),
        LazyInstantiatedContentSource {
            get_source: source.into(),
        }
        .cell()
        .into(),
    )
    .into()
}

/// see [create_node_rendered_source]
#[turbo_tasks::value]
struct NodeRenderContentSource {
    server_root: FileSystemPathVc,
    path_regex: PathRegexVc,
    renderer: NodeRendererVc,
    chunking_context: ChunkingContextVc,
    runtime_entries: EcmascriptChunkPlaceablesVc,
    intermediate_output_path: FileSystemPathVc,
}

impl NodeRenderContentSource {
    /// Checks if a path matches the regular expression
    async fn is_matching_path(&self, path: &str) -> Result<bool> {
        Ok(self.path_regex.await?.is_match(path))
    }

    /// Matches a path with the regular expression and returns a JSON object
    /// with the named captures
    async fn get_matches(&self, path: &str) -> Result<Option<IndexMap<String, String>>> {
        Ok(self.path_regex.await?.get_matches(path))
    }
}

#[turbo_tasks::value_impl]
impl GetContentSource for NodeRenderContentSource {
    /// Returns the [ContentSource] that the serves all referenced external
    /// assets. This is wrapped into [LazyInstantiatedContentSource].
    #[turbo_tasks::function]
    fn content_source(&self) -> ContentSourceVc {
        AssetGraphContentSourceVc::new_lazy_multiple(
            self.server_root,
            external_asset_entrypoints(
                self.renderer.module(),
                self.runtime_entries,
                self.chunking_context,
                self.intermediate_output_path,
            ),
        )
        .into()
    }
}

#[turbo_tasks::value_impl]
impl ContentSource for NodeRenderContentSource {
    #[turbo_tasks::function]
    async fn vary(&self, path: &str) -> Result<ContentSourceDataVaryVc> {
        Ok(if self.is_matching_path(path).await? {
            ContentSourceDataVary {
                method: true,
                url: true,
                headers: Some(ContentSourceDataFilter::All),
                query: Some(ContentSourceDataFilter::All),
                ..Default::default()
            }
            .cell()
        } else {
            ContentSourceDataVary::default().cell()
        })
    }

    #[turbo_tasks::function]
    async fn get(
        &self,
        path: &str,
        data: turbo_tasks::Value<ContentSourceData>,
    ) -> Result<ContentSourceResultVc> {
        Ok(if let Some(params) = self.get_matches(path).await? {
            ContentSourceResult::Static(
                render_static(
                    self.server_root.join(path),
                    self.renderer.module(),
                    self.runtime_entries,
                    self.chunking_context,
                    self.intermediate_output_path,
                    RenderData {
                        params,
                        method: data
                            .method
                            .clone()
                            .ok_or_else(|| anyhow!("method needs to be provided"))?,
                        url: data
                            .url
                            .clone()
                            .ok_or_else(|| anyhow!("url needs to be provided"))?,
                        query: data.query.clone(),
                        headers: data.headers.clone(),
                    }
                    .cell(),
                )
                .into(),
            )
            .cell()
        } else {
            ContentSourceResult::NotFound.cell()
        })
    }

    #[turbo_tasks::function]
    fn get_by_id(&self, _id: &str) -> ContentSourceResultVc {
        // TODO allow to subscribe to the content
        ContentSourceResult::NotFound.cell()
    }
}