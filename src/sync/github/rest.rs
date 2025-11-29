//! GitHub REST API client
//!
//! Handles issues, labels, and other REST endpoints.

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};

const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT_VALUE: &str = "mx-sync/0.1";

/// GitHub REST API client
pub struct RestClient {
    client: Client,
    token: String,
}

impl RestClient {
    /// Create a new REST client with the given token
    pub fn new(token: String) -> Result<Self> {
        let client = Client::builder()
            .default_headers(Self::default_headers(&token)?)
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self { client, token })
    }

    fn default_headers(token: &str) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", token))
                .context("Invalid token format")?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
        headers.insert(
            "X-GitHub-Api-Version",
            HeaderValue::from_static("2022-11-28"),
        );
        Ok(headers)
    }

    /// List all issues in a repository (paginated)
    pub fn list_issues(&self, owner: &str, repo: &str, state: &str) -> Result<Vec<Issue>> {
        let mut all_issues = Vec::new();
        let mut page = 1;

        loop {
            let url = format!(
                "{}/repos/{}/{}/issues?state={}&per_page=100&page={}",
                GITHUB_API_BASE, owner, repo, state, page
            );

            let response: Vec<Issue> = self
                .client
                .get(&url)
                .send()
                .context("Failed to fetch issues")?
                .error_for_status()
                .context("GitHub API error")?
                .json()
                .context("Failed to parse issues response")?;

            if response.is_empty() {
                break;
            }

            let count = response.len();
            all_issues.extend(response);

            if count < 100 {
                break;
            }

            page += 1;
        }

        // Filter out pull requests (they show up in issues API)
        all_issues.retain(|issue| issue.pull_request.is_none());

        Ok(all_issues)
    }

    /// Get a single issue by number
    pub fn get_issue(&self, owner: &str, repo: &str, number: u64) -> Result<Issue> {
        let url = format!(
            "{}/repos/{}/{}/issues/{}",
            GITHUB_API_BASE, owner, repo, number
        );

        self.client
            .get(&url)
            .send()
            .context("Failed to fetch issue")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse issue response")
    }

    /// Create a new issue
    pub fn create_issue(&self, owner: &str, repo: &str, req: &CreateIssueRequest) -> Result<Issue> {
        let url = format!("{}/repos/{}/{}/issues", GITHUB_API_BASE, owner, repo);

        self.client
            .post(&url)
            .json(req)
            .send()
            .context("Failed to create issue")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse create issue response")
    }

    /// Update an existing issue
    pub fn update_issue(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        req: &UpdateIssueRequest,
    ) -> Result<Issue> {
        let url = format!(
            "{}/repos/{}/{}/issues/{}",
            GITHUB_API_BASE, owner, repo, number
        );

        self.client
            .patch(&url)
            .json(req)
            .send()
            .context("Failed to update issue")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse update issue response")
    }

    /// List all labels in a repository
    pub fn list_labels(&self, owner: &str, repo: &str) -> Result<Vec<Label>> {
        let mut all_labels = Vec::new();
        let mut page = 1;

        loop {
            let url = format!(
                "{}/repos/{}/{}/labels?per_page=100&page={}",
                GITHUB_API_BASE, owner, repo, page
            );

            let response: Vec<Label> = self
                .client
                .get(&url)
                .send()
                .context("Failed to fetch labels")?
                .error_for_status()
                .context("GitHub API error")?
                .json()
                .context("Failed to parse labels response")?;

            if response.is_empty() {
                break;
            }

            let count = response.len();
            all_labels.extend(response);

            if count < 100 {
                break;
            }

            page += 1;
        }

        Ok(all_labels)
    }

    /// Create a new label
    pub fn create_label(&self, owner: &str, repo: &str, req: &CreateLabelRequest) -> Result<Label> {
        let url = format!("{}/repos/{}/{}/labels", GITHUB_API_BASE, owner, repo);

        self.client
            .post(&url)
            .json(req)
            .send()
            .context("Failed to create label")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse create label response")
    }

    /// Update an existing label
    pub fn update_label(
        &self,
        owner: &str,
        repo: &str,
        name: &str,
        req: &UpdateLabelRequest,
    ) -> Result<Label> {
        let url = format!(
            "{}/repos/{}/{}/labels/{}",
            GITHUB_API_BASE,
            owner,
            repo,
            urlencoding::encode(name)
        );

        self.client
            .patch(&url)
            .json(req)
            .send()
            .context("Failed to update label")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse update label response")
    }

    /// List comments on an issue
    pub fn list_issue_comments(&self, owner: &str, repo: &str, number: u64) -> Result<Vec<Comment>> {
        let mut all_comments = Vec::new();
        let mut page = 1;

        loop {
            let url = format!(
                "{}/repos/{}/{}/issues/{}/comments?per_page=100&page={}",
                GITHUB_API_BASE, owner, repo, number, page
            );

            let response: Vec<Comment> = self
                .client
                .get(&url)
                .send()
                .context("Failed to fetch comments")?
                .error_for_status()
                .context("GitHub API error")?
                .json()
                .context("Failed to parse comments response")?;

            if response.is_empty() {
                break;
            }

            let count = response.len();
            all_comments.extend(response);

            if count < 100 {
                break;
            }

            page += 1;
        }

        Ok(all_comments)
    }

    /// Generic POST request with JSON body
    pub fn post_json<T: Serialize, R: for<'de> Deserialize<'de>>(&self, url: &str, body: &T) -> Result<R> {
        self.client
            .post(url)
            .json(body)
            .send()
            .context("Failed to execute POST request")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse JSON response")
    }
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// GitHub Issue
#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<LabelRef>,
    pub assignees: Vec<UserRef>,
    pub updated_at: String,
    pub pull_request: Option<PullRequestRef>,
}

impl Issue {
    pub fn label_names(&self) -> Vec<String> {
        self.labels.iter().map(|l| l.name.clone()).collect()
    }

    pub fn assignee_logins(&self) -> Vec<String> {
        self.assignees.iter().map(|a| a.login.clone()).collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelRef {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserRef {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequestRef {
    pub url: String,
}

/// GitHub Label
#[derive(Debug, Clone, Deserialize)]
pub struct Label {
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// GitHub Comment
#[derive(Debug, Clone, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub body: Option<String>,
    pub user: UserRef,
    pub created_at: String,
}

/// Request to create an issue
#[derive(Debug, Serialize)]
pub struct CreateIssueRequest {
    pub title: String,
    pub body: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<String>,
}

/// Request to update an issue
#[derive(Debug, Serialize)]
pub struct UpdateIssueRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Request to create a label
#[derive(Debug, Serialize)]
pub struct CreateLabelRequest {
    pub name: String,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Request to update a label
#[derive(Debug, Serialize)]
pub struct UpdateLabelRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
