import requests
from bs4 import BeautifulSoup
from concurrent.futures import ThreadPoolExecutor

base_url = "http://localhost:8080"

full_path = f"{base_url}/r/politics"

ctr = 0

def fetch_url(url):
    global ctr
    response = requests.get(url)
    ctr += 1
    print(f"Request count: {ctr}")
    return response

while full_path:
    response = requests.get(full_path)
    ctr += 1
    print(f"Request count: {ctr}")
    soup = BeautifulSoup(response.text, 'html.parser')
    comment_links = soup.find_all('a', class_='post_comments')
    comment_urls = [base_url + link['href'] for link in comment_links]
    with ThreadPoolExecutor(max_workers=10) as executor:
        executor.map(fetch_url, comment_urls)
    next_link = soup.find('a', accesskey='N')
    if next_link:
        full_path = base_url + next_link['href']
    else:
        break
