//! GitHub GraphQL API client
//!
//! Handles discussions and other GraphQL-only endpoints.

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::json;

const GITHUB_GRAPHQL_URL: &str = "https://api.github.com/graphql";
const USER_AGENT_VALUE: &str = "mx-sync/0.1";

/// GitHub GraphQL API client
pub struct GraphQLClient {
    client: Client,
}

impl GraphQLClient {
    /// Create a new GraphQL client with the given token
    pub fn new(token: &str) -> Result<Self> {
        let client = Client::builder()
            .default_headers(Self::default_headers(token)?)
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client })
    }

    fn default_headers(token: &str) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token))
                .context("Invalid token format")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        Ok(headers)
    }

    /// Execute a GraphQL query
    fn query<T: for<'de> Deserialize<'de>>(&self, query: &str, variables: serde_json::Value) -> Result<T> {
        let body = json!({
            "query": query,
            "variables": variables
        });

        let response = self
            .client
            .post(GITHUB_GRAPHQL_URL)
            .json(&body)
            .send()
            .context("Failed to execute GraphQL query")?;

        let status = response.status();
        let text = response.text().context("Failed to read response")?;

        if !status.is_success() {
            anyhow::bail!("GraphQL request failed ({}): {}", status, text);
        }

        let result: GraphQLResponse<T> = serde_json::from_str(&text)
            .with_context(|| format!("Failed to parse GraphQL response: {}", text))?;

        if let Some(errors) = result.errors {
            if !errors.is_empty() {
                let messages: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
                anyhow::bail!("GraphQL errors: {}", messages.join(", "));
            }
        }

        result.data.context("No data in GraphQL response")
    }

    /// Get repository ID (needed for mutations)
    pub fn get_repository_id(&self, owner: &str, repo: &str) -> Result<String> {
        let query = r#"
            query($owner: String!, $repo: String!) {
                repository(owner: $owner, name: $repo) {
                    id
                }
            }
        "#;

        let variables = json!({
            "owner": owner,
            "repo": repo
        });

        let data: RepositoryIdResponse = self.query(query, variables)?;
        Ok(data.repository.id)
    }

    /// List discussion categories for a repository
    pub fn list_discussion_categories(&self, owner: &str, repo: &str) -> Result<Vec<DiscussionCategory>> {
        let query = r#"
            query($owner: String!, $repo: String!) {
                repository(owner: $owner, name: $repo) {
                    discussionCategories(first: 100) {
                        nodes {
                            id
                            name
                            slug
                        }
                    }
                }
            }
        "#;

        let variables = json!({
            "owner": owner,
            "repo": repo
        });

        let data: DiscussionCategoriesResponse = self.query(query, variables)?;
        Ok(data.repository.discussion_categories.nodes)
    }

    /// List all discussions in a repository
    pub fn list_discussions(&self, owner: &str, repo: &str) -> Result<Vec<Discussion>> {
        let mut all_discussions = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let query = r#"
                query($owner: String!, $repo: String!, $cursor: String) {
                    repository(owner: $owner, name: $repo) {
                        discussions(first: 100, after: $cursor) {
                            pageInfo {
                                hasNextPage
                                endCursor
                            }
                            nodes {
                                id
                                number
                                title
                                body
                                updatedAt
                                category {
                                    id
                                    name
                                    slug
                                }
                                labels(first: 100) {
                                    nodes {
                                        name
                                    }
                                }
                                comments(first: 100) {
                                    nodes {
                                        id
                                        body
                                        createdAt
                                        author {
                                            login
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            "#;

            let variables = json!({
                "owner": owner,
                "repo": repo,
                "cursor": cursor
            });

            let data: DiscussionsResponse = self.query(query, variables)?;
            let discussions = data.repository.discussions;

            all_discussions.extend(discussions.nodes);

            if discussions.page_info.has_next_page {
                cursor = discussions.page_info.end_cursor;
            } else {
                break;
            }
        }

        Ok(all_discussions)
    }

    /// Create a new discussion
    pub fn create_discussion(
        &self,
        repo_id: &str,
        category_id: &str,
        title: &str,
        body: &str,
    ) -> Result<Discussion> {
        let query = r#"
            mutation($repoId: ID!, $categoryId: ID!, $title: String!, $body: String!) {
                createDiscussion(input: {
                    repositoryId: $repoId,
                    categoryId: $categoryId,
                    title: $title,
                    body: $body
                }) {
                    discussion {
                        id
                        number
                        title
                        body
                        updatedAt
                        category {
                            id
                            name
                            slug
                        }
                        labels(first: 100) {
                            nodes {
                                name
                            }
                        }
                        comments(first: 100) {
                            nodes {
                                id
                                body
                                createdAt
                                author {
                                    login
                                }
                            }
                        }
                    }
                }
            }
        "#;

        let variables = json!({
            "repoId": repo_id,
            "categoryId": category_id,
            "title": title,
            "body": body
        });

        let data: CreateDiscussionResponse = self.query(query, variables)?;
        Ok(data.create_discussion.discussion)
    }

    /// Update an existing discussion
    pub fn update_discussion(
        &self,
        discussion_id: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<Discussion> {
        let query = r#"
            mutation($discussionId: ID!, $title: String, $body: String) {
                updateDiscussion(input: {
                    discussionId: $discussionId,
                    title: $title,
                    body: $body
                }) {
                    discussion {
                        id
                        number
                        title
                        body
                        updatedAt
                        category {
                            id
                            name
                            slug
                        }
                        labels(first: 100) {
                            nodes {
                                name
                            }
                        }
                        comments(first: 100) {
                            nodes {
                                id
                                body
                                createdAt
                                author {
                                    login
                                }
                            }
                        }
                    }
                }
            }
        "#;

        let variables = json!({
            "discussionId": discussion_id,
            "title": title,
            "body": body
        });

        let data: UpdateDiscussionResponse = self.query(query, variables)?;
        Ok(data.update_discussion.discussion)
    }

    /// Get discussion node ID from discussion number
    pub fn get_discussion_id(&self, owner: &str, repo: &str, number: u64) -> Result<String> {
        let query = r#"
            query($owner: String!, $repo: String!, $number: Int!) {
                repository(owner: $owner, name: $repo) {
                    discussion(number: $number) {
                        id
                    }
                }
            }
        "#;

        let variables = json!({
            "owner": owner,
            "repo": repo,
            "number": number
        });

        let data: DiscussionIdResponse = self.query(query, variables)?;
        Ok(data.repository.discussion.id)
    }

    /// Delete a discussion by its node ID
    pub fn delete_discussion(&self, id: &str) -> Result<()> {
        let query = r#"
            mutation($id: ID!) {
                deleteDiscussion(input: {id: $id}) {
                    clientMutationId
                }
            }
        "#;

        let variables = json!({
            "id": id
        });

        // We don't need the response data, just check for errors
        let _: DeleteDiscussionResponse = self.query(query, variables)?;
        Ok(())
    }

    /// Add a comment to a discussion
    pub fn add_discussion_comment(&self, discussion_id: &str, body: &str) -> Result<DiscussionCommentCreated> {
        let query = r#"
            mutation($discussionId: ID!, $body: String!) {
                addDiscussionComment(input: {discussionId: $discussionId, body: $body}) {
                    comment {
                        id
                        url
                    }
                }
            }
        "#;

        let variables = json!({
            "discussionId": discussion_id,
            "body": body
        });

        let data: AddDiscussionCommentResponse = self.query(query, variables)?;
        Ok(data.add_discussion_comment.comment)
    }
}

// ============================================================================
// Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct RepositoryIdResponse {
    repository: RepositoryId,
}

#[derive(Debug, Deserialize)]
struct RepositoryId {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DiscussionCategoriesResponse {
    repository: DiscussionCategoriesRepo,
}

#[derive(Debug, Deserialize)]
struct DiscussionCategoriesRepo {
    #[serde(rename = "discussionCategories")]
    discussion_categories: DiscussionCategoriesNodes,
}

#[derive(Debug, Deserialize)]
struct DiscussionCategoriesNodes {
    nodes: Vec<DiscussionCategory>,
}

/// Discussion category
#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionCategory {
    pub id: String,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Deserialize)]
struct DiscussionsResponse {
    repository: DiscussionsRepo,
}

#[derive(Debug, Deserialize)]
struct DiscussionsRepo {
    discussions: DiscussionsConnection,
}

#[derive(Debug, Deserialize)]
struct DiscussionsConnection {
    #[serde(rename = "pageInfo")]
    page_info: PageInfo,
    nodes: Vec<Discussion>,
}

#[derive(Debug, Deserialize)]
struct PageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

/// GitHub Discussion
#[derive(Debug, Clone, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    pub category: DiscussionCategoryRef,
    pub labels: LabelsConnection,
    pub comments: CommentsConnection,
}

impl Discussion {
    pub fn label_names(&self) -> Vec<String> {
        self.labels.nodes.iter().map(|l| l.name.clone()).collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionCategoryRef {
    pub id: String,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelsConnection {
    pub nodes: Vec<LabelNode>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelNode {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentsConnection {
    pub nodes: Vec<DiscussionComment>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscussionComment {
    pub id: String,
    pub body: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub author: Option<Author>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Author {
    pub login: String,
}

#[derive(Debug, Deserialize)]
struct CreateDiscussionResponse {
    #[serde(rename = "createDiscussion")]
    create_discussion: CreateDiscussionPayload,
}

#[derive(Debug, Deserialize)]
struct CreateDiscussionPayload {
    discussion: Discussion,
}

#[derive(Debug, Deserialize)]
struct UpdateDiscussionResponse {
    #[serde(rename = "updateDiscussion")]
    update_discussion: UpdateDiscussionPayload,
}

#[derive(Debug, Deserialize)]
struct UpdateDiscussionPayload {
    discussion: Discussion,
}

#[derive(Debug, Deserialize)]
struct DiscussionIdResponse {
    repository: DiscussionIdRepo,
}

#[derive(Debug, Deserialize)]
struct DiscussionIdRepo {
    discussion: DiscussionIdData,
}

#[derive(Debug, Deserialize)]
struct DiscussionIdData {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DeleteDiscussionResponse {
    #[serde(rename = "deleteDiscussion")]
    delete_discussion: DeleteDiscussionPayload,
}

#[derive(Debug, Deserialize)]
struct DeleteDiscussionPayload {
    #[serde(rename = "clientMutationId")]
    client_mutation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AddDiscussionCommentResponse {
    #[serde(rename = "addDiscussionComment")]
    add_discussion_comment: AddDiscussionCommentPayload,
}

#[derive(Debug, Deserialize)]
struct AddDiscussionCommentPayload {
    comment: DiscussionCommentCreated,
}

/// Discussion comment created via mutation
#[derive(Debug, Deserialize)]
pub struct DiscussionCommentCreated {
    pub id: String,
    pub url: String,
}
